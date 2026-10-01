use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use image::{RgbaImage, imageops};
use resvg::{tiny_skia, usvg};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{ContextId, ImportMem, Renderer};
use smithay::utils::{Physical, Rectangle};

use super::window_texture::WindowTexture;

const RASTER_SIZE: u32 = 64;
const WALK_DEPTH: usize = 6;

/// How long a request may stay `Pending` before it stops holding the
/// compositor awake.
///
/// A pending icon contributes to [`AppIconCache::has_pending`], which is
/// OR-ed into the per-output animation flag, so a request that never resolves
/// pins the redraw loop at full refresh rate forever. The loader normally
/// answers within one or two frames; anything past this deadline is a wedged
/// or dead loader thread, and a missing icon is a far better outcome than a
/// compositor that never sleeps again.
const ICON_PENDING_DEADLINE: Duration = Duration::from_secs(5);

/// How long a resolved entry survives after the last frame that drew it.
const ICON_CACHE_TTL: Duration = Duration::from_secs(300);

/// Hard ceiling on resolved entries, independent of [`ICON_CACHE_TTL`].
///
/// `app_id` is client-controlled and a client may set a fresh one on every
/// commit, so the TTL alone would still let a burst allocate an unbounded
/// number of 64x64 textures before the first sweep reclaims any of them.
const ICON_CACHE_CAPACITY: usize = 256;

struct Raster {
    pixels: Vec<u8>,
}

enum Result {
    Loaded { app_id: String, raster: Raster },
    Missing { app_id: String },
}

struct Loader {
    jobs: Sender<String>,
    results: Receiver<Result>,
}

impl Loader {
    fn spawn() -> Self {
        let (jobs, job_rx) = channel::<String>();
        let (result_tx, results) = channel();
        thread::Builder::new()
            .name("halley-app-icon".into())
            .spawn(move || {
                while let Ok(app_id) = job_rx.recv() {
                    let result = resolve_icon_path(&app_id)
                        .and_then(|path| load_raster(&path))
                        .map(|raster| Result::Loaded {
                            app_id: app_id.clone(),
                            raster,
                        })
                        .unwrap_or(Result::Missing { app_id });
                    if result_tx.send(result).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn app-icon loader");
        Self { jobs, results }
    }
}

enum State {
    Pending { requested_at: Instant },
    Missing,
    Ready(WindowTexture),
}

struct Entry {
    state: State,
    last_used: Instant,
}

#[derive(Default)]
pub(super) struct AppIconCache {
    context: Option<ContextId<GlesTexture>>,
    loader: Option<Loader>,
    entries: HashMap<String, Entry>,
}

impl AppIconCache {
    /// Whether any request is still waiting on the loader and is recent enough
    /// to be worth animating for.
    ///
    /// Requests past [`ICON_PENDING_DEADLINE`] are deliberately excluded even
    /// though they remain `Pending`: see that constant for why.
    pub(super) fn has_pending(&self) -> bool {
        let now = Instant::now();
        self.entries.values().any(|entry| match entry.state {
            State::Pending { requested_at } => {
                now.saturating_duration_since(requested_at) < ICON_PENDING_DEADLINE
            }
            State::Missing | State::Ready(_) => false,
        })
    }

    /// Per-frame maintenance: collect finished loads and age out stale entries.
    ///
    /// This must run every frame regardless of whether any icon is drawn.
    /// Draining only from [`Self::request`] meant a request whose node stopped
    /// being drawn before its result arrived stayed `Pending` forever, and
    /// [`Self::has_pending`] then held the whole compositor at full refresh
    /// rate for the rest of the session.
    pub(super) fn poll(&mut self, renderer: &mut GlesRenderer) {
        self.refresh_context(renderer);
        self.drain(renderer);
        self.sweep();
    }

    pub(super) fn request(&mut self, renderer: &mut GlesRenderer, app_id: &str) {
        self.refresh_context(renderer);
        let now = Instant::now();
        if let Some(entry) = self.entries.get_mut(app_id) {
            entry.last_used = now;
            return;
        }
        self.entries.insert(
            app_id.to_string(),
            Entry {
                state: State::Pending { requested_at: now },
                last_used: now,
            },
        );
        let loader = self.loader.get_or_insert_with(Loader::spawn);
        let _ = loader.jobs.send(app_id.to_string());
    }

    pub(super) fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        id: Id,
        app_id: &str,
        destination: Rectangle<i32, Physical>,
        alpha: f32,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        self.request(renderer, app_id);
        match self.entries.get(app_id).map(|entry| &entry.state) {
            Some(State::Ready(icon)) => Some(icon.render_element(id, destination, alpha)),
            Some(State::Pending { .. } | State::Missing) | None => None,
        }
    }

    /// Drops entries nothing has drawn recently, and enforces a hard ceiling.
    ///
    /// `Pending` entries are never evicted here: removing one would let the
    /// next `request` re-issue the same job and start the deadline over, so a
    /// permanently unresolvable icon could re-wedge the animation flag on a
    /// loop.
    fn sweep(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, entry| {
            matches!(entry.state, State::Pending { .. })
                || now.saturating_duration_since(entry.last_used) < ICON_CACHE_TTL
        });
        while self.entries.len() > ICON_CACHE_CAPACITY {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(_, entry)| !matches!(entry.state, State::Pending { .. }))
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(app_id, _)| app_id.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn refresh_context(&mut self, renderer: &GlesRenderer) {
        let context = renderer.context_id();
        if self
            .context
            .as_ref()
            .is_some_and(|current| *current == context)
        {
            return;
        }
        self.context = Some(context);
        self.entries.clear();
    }

    fn drain(&mut self, renderer: &mut GlesRenderer) {
        let Some(loader) = self.loader.as_ref() else {
            return;
        };
        while let Ok(result) = loader.results.try_recv() {
            let (app_id, state) = match result {
                Result::Loaded { app_id, raster } => {
                    let state = renderer
                        .import_memory(
                            &raster.pixels,
                            Fourcc::Abgr8888,
                            (RASTER_SIZE as i32, RASTER_SIZE as i32).into(),
                            false,
                        )
                        .ok()
                        .map(|texture| {
                            State::Ready(WindowTexture {
                                texture,
                                context: renderer.context_id(),
                            })
                        })
                        .unwrap_or(State::Missing);
                    (app_id, state)
                }
                Result::Missing { app_id } => (app_id, State::Missing),
            };
            // A result can outlive its request: the sweep drops entries nothing
            // has drawn for a while, and the loader may still answer afterwards.
            // Preserve the original `last_used` when the entry survives so a
            // late answer cannot extend an unused icon's lifetime.
            let last_used = self
                .entries
                .get(&app_id)
                .map_or_else(Instant::now, |entry| entry.last_used);
            self.entries.insert(app_id, Entry { state, last_used });
        }
    }
}

fn resolve_icon_path(app_id: &str) -> Option<PathBuf> {
    let mut names = vec![app_id.to_string()];
    if let Some(tail) = app_id.rsplit(['.', '/']).next()
        && !tail.is_empty()
        && tail != app_id
    {
        names.push(tail.to_string());
    }
    if let Some(icon) = desktop_icon(app_id) {
        names.push(icon);
    }
    names.into_iter().find_map(|name| find_icon(&name))
}

fn desktop_icon(app_id: &str) -> Option<String> {
    for directory in data_roots()
        .into_iter()
        .map(|root| root.join("applications"))
    {
        let exact = directory.join(format!("{app_id}.desktop"));
        if let Some(icon) = parse_desktop_entry(&exact).map(|entry| entry.icon) {
            return Some(icon);
        }
        let mut found = None;
        walk(&directory, 2, &mut |path| {
            if found.is_some() || path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
                return;
            }
            let stem_matches = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(app_id));
            let entry = parse_desktop_entry(path);
            let startup_matches = entry
                .as_ref()
                .and_then(|entry| entry.startup_wm_class.as_deref())
                .is_some_and(|class| class.eq_ignore_ascii_case(app_id));
            if stem_matches || startup_matches {
                found = entry.map(|entry| entry.icon);
            }
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

struct DesktopEntry {
    icon: String,
    startup_wm_class: Option<String>,
}

fn parse_desktop_entry(path: &Path) -> Option<DesktopEntry> {
    let text = fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let mut icon = None;
    let mut startup_wm_class = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            if in_entry {
                break;
            }
            in_entry = line.eq_ignore_ascii_case("[Desktop Entry]");
        } else if in_entry {
            if let Some(value) = line.strip_prefix("Icon=") {
                icon = Some(value.trim_matches(['"', '\'', ' ']).to_string());
            } else if let Some(value) = line.strip_prefix("StartupWMClass=") {
                startup_wm_class = Some(value.trim_matches(['"', '\'', ' ']).to_string());
            }
        }
    }
    Some(DesktopEntry {
        icon: icon?,
        startup_wm_class,
    })
}

fn find_icon(name: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(name);
    if direct.is_file() {
        return Some(direct);
    }
    let mut best: Option<(i32, PathBuf)> = None;
    for root in icon_roots() {
        walk(&root, WALK_DEPTH, &mut |path| {
            let Some(score) = icon_score(path, name) else {
                return;
            };
            if best.as_ref().is_none_or(|(current, _)| score < *current) {
                best = Some((score, path.to_path_buf()));
            }
        });
    }
    best.map(|(_, path)| path)
}

fn icon_score(path: &Path, name: &str) -> Option<i32> {
    if path.file_stem()?.to_str()? != name {
        return None;
    }
    let format = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "svg" => 0,
        "png" => 30,
        "jpg" | "jpeg" => 50,
        _ => return None,
    };
    let size = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .find_map(|part| {
            let (width, height) = part.split_once('x')?;
            Some(width.parse::<i32>().ok()?.min(height.parse::<i32>().ok()?))
        })
        .map(|size| (size - RASTER_SIZE as i32).abs())
        .unwrap_or(20);
    Some(format + size)
}

fn data_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        roots.push(path.into());
    } else if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share"));
    }
    for path in env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
        .split(':')
        .filter(|path| !path.is_empty())
    {
        roots.push(path.into());
    }
    roots
}

fn icon_roots() -> Vec<PathBuf> {
    let mut roots = data_roots()
        .into_iter()
        .map(|root| root.join("icons"))
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    if let Some(home) = env::var_os("HOME") {
        let legacy = PathBuf::from(home).join(".icons");
        if legacy.is_dir() {
            roots.push(legacy);
        }
    }
    let pixmaps = PathBuf::from("/usr/share/pixmaps");
    if pixmaps.is_dir() {
        roots.push(pixmaps);
    }
    roots
}

fn walk(root: &Path, max_depth: usize, visit: &mut dyn FnMut(&Path)) {
    fn recurse(path: &Path, depth: usize, max_depth: usize, visit: &mut dyn FnMut(&Path)) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < max_depth {
                    recurse(&path, depth + 1, max_depth, visit);
                }
            } else {
                visit(&path);
            }
        }
    }
    if root.is_dir() {
        recurse(root, 0, max_depth, visit);
    }
}

fn load_raster(path: &Path) -> Option<Raster> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "svg" => load_svg(path),
        "png" | "jpg" | "jpeg" => {
            let source = image::open(path).ok()?.to_rgba8();
            let resized = imageops::thumbnail(&source, RASTER_SIZE, RASTER_SIZE);
            let mut canvas = RgbaImage::new(RASTER_SIZE, RASTER_SIZE);
            imageops::overlay(
                &mut canvas,
                &resized,
                ((RASTER_SIZE - resized.width()) / 2) as i64,
                ((RASTER_SIZE - resized.height()) / 2) as i64,
            );
            Some(Raster {
                pixels: canvas.into_vec(),
            })
        }
        _ => None,
    }
}

fn load_svg(path: &Path) -> Option<Raster> {
    let options = usvg::Options {
        resources_dir: path.parent().map(Path::to_path_buf),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(&fs::read(path).ok()?, &options).ok()?;
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(RASTER_SIZE, RASTER_SIZE)?;
    let scale =
        (RASTER_SIZE as f32 / size.width() as f32).min(RASTER_SIZE as f32 / size.height() as f32);
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(
        (RASTER_SIZE as f32 - size.width() as f32 * scale) / 2.0,
        (RASTER_SIZE as f32 - size.height() as f32 * scale) / 2.0,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut pixels = pixmap.data().to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            pixel[0] = ((u32::from(pixel[0]) * 255) / alpha).min(255) as u8;
            pixel[1] = ((u32::from(pixel[1]) * 255) / alpha).min(255) as u8;
            pixel[2] = ((u32::from(pixel[2]) * 255) / alpha).min(255) as u8;
        }
    }
    Some(Raster { pixels })
}
