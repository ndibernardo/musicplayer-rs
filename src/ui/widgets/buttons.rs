use gtk4::Button;
use gtk4::MenuButton;
use gtk4::prelude::*;

use crate::ui::widgets::icons::AppIcon;

/// A frameless icon button — the app's standard look for inline actions.
pub fn flat_icon_button(icon: AppIcon, tooltip: &str) -> Button {
    let button = Button::from_icon_name(icon.name());
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button
}

/// A frameless icon menu button, visually matching [`flat_icon_button`].
/// `MenuButton` wraps its own internal toggle, so a `.flat` class on the
/// outer widget doesn't reach it the way it does a plain `Button` —
/// `has-frame` is the property both widgets share for this look. The caller
/// attaches the popover.
pub fn flat_menu_button(icon: AppIcon, tooltip: &str) -> MenuButton {
    let button = MenuButton::new();
    button.set_icon_name(icon.name());
    button.set_tooltip_text(Some(tooltip));
    button.set_has_frame(false);
    button
}
