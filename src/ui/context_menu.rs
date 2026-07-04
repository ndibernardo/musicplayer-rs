use gtk4::Button;
use gtk4::Popover;
use gtk4::Widget;
use gtk4::gdk::Rectangle;
use gtk4::prelude::*;

/// Shows a one-item "Add to Queue" popover anchored at `(x, y)` — widget-local
/// coordinates, as reported by a `GestureClick` attached to `parent`. The
/// popover detaches itself from `parent` once closed, so repeated right-clicks
/// don't accumulate stale popovers.
pub fn show_add_to_queue_menu(parent: &Widget, x: f64, y: f64, on_add: impl Fn() + 'static) {
    let popover = Popover::new();
    popover.set_parent(parent);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&Rectangle::new(x as i32, y as i32, 1, 1)));

    let button = Button::with_label("Add to Queue");
    button.add_css_class("flat");
    popover.set_child(Some(&button));

    let for_click = popover.clone();
    button.connect_clicked(move |_| {
        on_add();
        for_click.popdown();
    });
    popover.connect_closed(|pop| pop.unparent());

    popover.popup();
}
