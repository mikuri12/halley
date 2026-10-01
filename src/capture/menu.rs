use smithay::utils::{Logical, Point, Rectangle};

const BAR_WIDTH: i32 = 420;
const BAR_HEIGHT: i32 = 80;
const BAR_BOTTOM_MARGIN: i32 = 24;
const ITEM_PADDING: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenshotMode {
    Region,
    Screen,
    Window,
}

impl ScreenshotMode {
    pub const ALL: [Self; 3] = [Self::Region, Self::Screen, Self::Window];
}

#[derive(Clone, Debug)]
pub struct ScreenshotMenu {
    output_name: String,
    output_geometry: Rectangle<i32, Logical>,
    selected: usize,
    hovered: Option<usize>,
    window_available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuLayout {
    pub bar: Rectangle<i32, Logical>,
    pub items: [Rectangle<i32, Logical>; 3],
}

impl ScreenshotMenu {
    pub fn new(
        output_name: String,
        output_geometry: Rectangle<i32, Logical>,
        window_available: bool,
    ) -> Self {
        Self {
            output_name,
            output_geometry,
            selected: 0,
            hovered: None,
            window_available,
        }
    }

    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    pub fn output_geometry(&self) -> Rectangle<i32, Logical> {
        self.output_geometry
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    pub fn window_available(&self) -> bool {
        self.window_available
    }

    pub fn selected_mode(&self) -> ScreenshotMode {
        ScreenshotMode::ALL[self.selected]
    }

    pub fn hit_test(&self, position: Point<f64, Logical>) -> Option<usize> {
        let position = position.to_i32_round();
        layout(self.output_geometry)
            .items
            .iter()
            .position(|item| item.contains(position))
            .filter(|index| self.is_enabled(*index))
    }

    pub fn hover(&mut self, position: Point<f64, Logical>) -> bool {
        let hovered = self.hit_test(position);
        let changed = self.hovered != hovered;
        self.hovered = hovered;
        if let Some(index) = hovered {
            self.selected = index;
        }
        changed
    }

    pub fn move_selection(&mut self, delta: i32) -> bool {
        let direction = if delta < 0 { -1 } else { 1 };
        let count = ScreenshotMode::ALL.len() as i32;
        let mut candidate = self.selected as i32;
        for _ in 0..count {
            candidate = (candidate + direction).rem_euclid(count);
            if self.is_enabled(candidate as usize) {
                self.selected = candidate as usize;
                self.hovered = Some(self.selected);
                return true;
            }
        }
        false
    }

    fn is_enabled(&self, index: usize) -> bool {
        ScreenshotMode::ALL
            .get(index)
            .is_some_and(|mode| *mode != ScreenshotMode::Window || self.window_available)
    }
}

pub fn layout(output: Rectangle<i32, Logical>) -> MenuLayout {
    let width = BAR_WIDTH.min(output.size.w.max(1));
    let height = BAR_HEIGHT.min(output.size.h.max(1));
    let x = output.loc.x + (output.size.w - width) / 2;
    let y = output.loc.y + (output.size.h - height - BAR_BOTTOM_MARGIN).max(0);
    let bar = Rectangle::new((x, y).into(), (width, height).into());
    let slot_width = width / 3;
    let item = |index: i32| {
        let left = x + index * slot_width;
        let right = if index == 2 {
            x + width
        } else {
            left + slot_width
        };
        let horizontal_padding = ITEM_PADDING.min((right - left).max(0) / 2);
        let vertical_padding = ITEM_PADDING.min(height.max(0) / 2);
        Rectangle::new(
            (left + horizontal_padding, y + vertical_padding).into(),
            (
                right - left - horizontal_padding * 2,
                height - vertical_padding * 2,
            )
                .into(),
        )
    };
    MenuLayout {
        bar,
        items: [item(0), item(1), item(2)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> Rectangle<i32, Logical> {
        Rectangle::new((1920, 0).into(), (1920, 1080).into())
    }

    fn center(rectangle: Rectangle<i32, Logical>) -> Point<f64, Logical> {
        Point::from((
            f64::from(rectangle.loc.x) + f64::from(rectangle.size.w) / 2.0,
            f64::from(rectangle.loc.y) + f64::from(rectangle.size.h) / 2.0,
        ))
    }

    #[test]
    fn layout_is_centered_near_the_bottom_of_its_output() {
        let layout = layout(output());
        assert_eq!(
            layout.bar,
            Rectangle::new((2670, 976).into(), (420, 80).into())
        );
        assert!(
            layout
                .items
                .iter()
                .all(|item| layout.bar.contains_rect(*item))
        );
    }

    #[test]
    fn hit_testing_uses_the_same_layout_as_rendering() {
        let menu = ScreenshotMenu::new("DP-2".to_string(), output(), true);
        let layout = layout(output());
        for (index, item) in layout.items.into_iter().enumerate() {
            assert_eq!(menu.hit_test(center(item)), Some(index), "item {index}");
        }
        assert_eq!(menu.hit_test((2000.0, 200.0).into()), None);
    }

    #[test]
    fn keyboard_navigation_wraps_and_skips_an_unavailable_window() {
        let mut menu = ScreenshotMenu::new("DP-2".to_string(), output(), false);
        assert!(menu.move_selection(-1));
        assert_eq!(menu.selected_mode(), ScreenshotMode::Screen);
        assert!(menu.move_selection(1));
        assert_eq!(menu.selected_mode(), ScreenshotMode::Region);
    }

    #[test]
    fn unavailable_window_does_not_hover() {
        let mut menu = ScreenshotMenu::new("DP-2".to_string(), output(), false);
        let window = center(layout(output()).items[2]);
        assert!(!menu.hover(window));
        assert_eq!(menu.hovered(), None);
        assert_eq!(menu.selected_mode(), ScreenshotMode::Region);
    }
}
