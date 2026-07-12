use std::rc::Rc;

use gtk4::Box as GtkBox;
use gtk4::Expander;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::ScrolledWindow;
use gtk4::prelude::*;

use crate::library::db::LibraryFolder;
use crate::ui::style;
use crate::ui::style::Margins;
use crate::ui::style::spacing;
use crate::ui::widgets::AppIcon;
use crate::ui::widgets::Callback;
use crate::ui::widgets::CollapsibleSection;
use crate::ui::widgets::ValueList;
use crate::ui::widgets::body_label;
use crate::ui::widgets::flat_icon_button;

/// The watched-folders browser: a collapsible section listing every watched
/// folder with an inline remove button per row. Collapses when the last
/// folder is removed, matching every other sidebar section.
#[derive(Clone)]
pub struct FolderList {
    section: CollapsibleSection,
    list: ValueList<LibraryFolder>,
    on_remove: Rc<Callback<LibraryFolder>>,
}

impl FolderList {
    pub fn new() -> Self {
        let on_remove: Rc<Callback<LibraryFolder>> = Rc::new(Callback::new());

        let render_remove = Rc::clone(&on_remove);
        let list = ValueList::new(gtk4::SelectionMode::None, move |folder: &LibraryFolder| {
            folder_row(folder, &render_remove)
        });

        let scrolled = ScrolledWindow::new();
        scrolled.set_min_content_height(120);
        scrolled.set_child(Some(list.widget()));

        let section = CollapsibleSection::new("Watched Folders", &scrolled);

        Self {
            section,
            list,
            on_remove,
        }
    }

    /// Replaces the visible folders, collapsing the section when none remain.
    pub fn set_folders(&self, folders: Vec<LibraryFolder>) {
        let empty = self.list.set_items(folders);
        self.section.set_empty(empty);
    }

    /// Registers the callback invoked with the folder whose remove button was
    /// clicked. Removal itself is the caller's job — this list only displays.
    pub fn connect_remove_requested<F: Fn(LibraryFolder) + 'static>(&self, f: F) {
        self.on_remove.set(f);
    }

    pub fn widget(&self) -> &Expander {
        self.section.widget()
    }
}

fn folder_row(folder: &LibraryFolder, on_remove: &Rc<Callback<LibraryFolder>>) -> ListBoxRow {
    let path_label = body_label(folder.as_path().to_str().unwrap_or_default());
    path_label.set_hexpand(true);
    // Middle ellipsis: with paths, the leaf directory name matters more than
    // the middle of the path.
    path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    path_label.set_margin_start(spacing::M);

    let remove_btn = flat_icon_button(AppIcon::ListRemove, "Remove folder");
    style::set_margins(&remove_btn, Margins::none().end(spacing::S));
    {
        let on_remove = Rc::clone(on_remove);
        let folder = folder.clone();
        remove_btn.connect_clicked(move |_| on_remove.emit(folder.clone()));
    }

    let row_box = GtkBox::new(Orientation::Horizontal, 0);
    style::set_margins(&row_box, Margins::vertical(spacing::XS));
    row_box.append(&path_label);
    row_box.append(&remove_btn);

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
