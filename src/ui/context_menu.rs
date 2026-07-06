use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::Orientation;
use gtk4::Popover;
use gtk4::Widget;
use gtk4::gdk::Rectangle;
use gtk4::prelude::*;

/// One labeled action in a `show_context_menu` popover. The label is an
/// owned `String` (not `&'static str`) since a batch action's label carries a
/// selection count computed at click time, e.g. "Add 3 to Queue".
pub type ContextAction = (String, Box<dyn Fn()>);

/// Shows a popover of one flat button per `(label, action)` pair in `actions`,
/// anchored at `(x, y)` — widget-local coordinates, as reported by a
/// `GestureClick` attached to `parent`. The popover detaches itself from
/// `parent` once closed, so repeated right-clicks don't accumulate stale
/// popovers.
pub fn show_context_menu(parent: &Widget, x: f64, y: f64, actions: Vec<ContextAction>) {
    let popover = Popover::new();
    popover.set_parent(parent);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&Rectangle::new(x as i32, y as i32, 1, 1)));

    let list = GtkBox::new(Orientation::Vertical, 0);
    for (label, action) in actions {
        let button = Button::with_label(&label);
        button.add_css_class("flat");
        let for_click = popover.clone();
        button.connect_clicked(move |_| {
            action();
            for_click.popdown();
        });
        list.append(&button);
    }
    popover.set_child(Some(&list));
    popover.connect_closed(|pop| pop.unparent());

    popover.popup();
}
