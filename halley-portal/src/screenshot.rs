use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::fdo;
use zbus::interface;
use zbus::message::Header;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use halley_ipc::{ScreenshotResponse, ScreenshotTarget};

const SCREENSHOT_VERSION: u32 = 3;
const TARGET_SCREEN: u32 = 1;
const TARGET_WINDOW: u32 = 2;
const TARGET_AREA: u32 = 4;
const AVAILABLE_TARGETS: u32 = TARGET_SCREEN | TARGET_WINDOW | TARGET_AREA;

type Vardict = HashMap<String, OwnedValue>;

pub struct ScreenshotInterface {
    connection: Connection,
}

impl ScreenshotInterface {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl ScreenshotInterface {
    fn screenshot(
        &self,
        handle: OwnedObjectPath,
        app_id: &str,
        _parent_window: &str,
        options: Vardict,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<(u32, Vardict)> {
        let owner = crate::auth::frontend(&self.connection, &header)?;
        let _request = export_request(&self.connection, &handle, owner.as_str())?;
        let interactive = extract_bool(&options, "interactive").unwrap_or(false);
        let target = match extract_u32(&options, "target") {
            Some(TARGET_SCREEN) => ScreenshotTarget::Screen,
            Some(TARGET_WINDOW) => ScreenshotTarget::Window,
            Some(TARGET_AREA) => ScreenshotTarget::Area,
            None if interactive => ScreenshotTarget::Area,
            None => ScreenshotTarget::Screen,
            Some(_) => return Ok((2, Vardict::new())),
        };

        eventline::info!(
            "portal screenshot: app_id={app_id:?} interactive={interactive} target={target:?}"
        );
        match crate::compositor::screenshot(handle.to_string(), target) {
            Ok(ScreenshotResponse::Saved { path }) => {
                let mut results = Vardict::new();
                results.insert(
                    "uri".to_string(),
                    owned(Value::from(path_to_file_uri(&path)))?,
                );
                Ok((0, results))
            }
            Ok(ScreenshotResponse::Cancelled) => Ok((1, Vardict::new())),
            Ok(ScreenshotResponse::Failed { message }) | Err(message) => {
                eventline::warn!("portal screenshot failed: {message}");
                Ok((2, Vardict::new()))
            }
        }
    }

    fn pick_color(
        &self,
        handle: OwnedObjectPath,
        _app_id: &str,
        _parent_window: &str,
        _options: Vardict,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<(u32, Vardict)> {
        let owner = crate::auth::frontend(&self.connection, &header)?;
        let _request = export_request(&self.connection, &handle, owner.as_str())?;
        Ok((2, Vardict::new()))
    }

    #[zbus(property)]
    fn available_targets(&self) -> u32 {
        AVAILABLE_TARGETS
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        SCREENSHOT_VERSION
    }
}

struct RequestInterface {
    owner: String,
    request_handle: String,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestInterface {
    fn close(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        crate::auth::same_owner(
            header.sender().map(|name| name.as_str()).unwrap_or(""),
            &self.owner,
        )?;
        if let Err(err) = crate::compositor::cancel_screenshot(self.request_handle.clone()) {
            eventline::debug!("portal request close: {err}");
        }
        Ok(())
    }
}

struct RequestGuard {
    connection: Connection,
    handle: OwnedObjectPath,
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        let _ = self
            .connection
            .object_server()
            .remove::<RequestInterface, _>(self.handle.clone());
    }
}

fn export_request(
    connection: &Connection,
    handle: &OwnedObjectPath,
    owner: &str,
) -> fdo::Result<RequestGuard> {
    if !connection.object_server().at(
        handle.clone(),
        RequestInterface {
            request_handle: handle.to_string(),
            owner: owner.into(),
        },
    )? {
        return Err(fdo::Error::InvalidArgs("request already exists".into()));
    }
    Ok(RequestGuard {
        connection: connection.clone(),
        handle: handle.clone(),
    })
}

fn extract_u32(dict: &Vardict, key: &str) -> Option<u32> {
    match &**dict.get(key)? {
        Value::U32(value) => Some(*value),
        _ => None,
    }
}

fn extract_bool(dict: &Vardict, key: &str) -> Option<bool> {
    match &**dict.get(key)? {
        Value::Bool(value) => Some(*value),
        _ => None,
    }
}

fn owned(value: Value<'_>) -> fdo::Result<OwnedValue> {
    OwnedValue::try_from(value).map_err(|err| fdo::Error::Failed(err.to_string()))
}

fn path_to_file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for byte in path.bytes() {
        match byte {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(char::from(byte))
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_escapes_spaces_and_non_ascii_bytes() {
        assert_eq!(
            path_to_file_uri("/tmp/Halley shot-é.png"),
            "file:///tmp/Halley%20shot-%C3%A9.png"
        );
    }

    #[test]
    fn advertised_targets_match_implemented_targets() {
        assert_eq!(
            AVAILABLE_TARGETS,
            TARGET_SCREEN | TARGET_WINDOW | TARGET_AREA
        );
    }
}
