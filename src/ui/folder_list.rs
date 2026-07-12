use std::rc::Rc;

use gtk4::ScrolledWindow;
use gtk4::SelectionMode;
use gtk4::pango;

use crate::library::db::LibraryFolder;
use crate::ui::widgets::Callback;
use crate::ui::widgets::NavTree;
use crate::ui::widgets::show_remove_popover;

/// The one and only category in the tree — a single "Watched Folders" row.
const WATCHED_FOLDERS: usize = 0;

/// The watched-folders browser: a single-category nav tree listing every
/// watched folder. Right-click a folder row to remove it — a `GtkTreeView`
/// cell can't host a real per-row button widget the way the `ListBox` row
/// this replaced could.
#[derive(Clone)]
pub struct FolderList {
    scrolled: ScrolledWindow,
    tree: Rc<NavTree<LibraryFolder>>,
    on_remove: Rc<Callback<LibraryFolder>>,
}

impl FolderList {
    pub fn new() -> Self {
        let render: Rc<dyn Fn(&LibraryFolder) -> String> = Rc::new(|folder: &LibraryFolder| {
            folder.as_path().to_str().unwrap_or_default().to_owned()
        });
        // Middle-ellipsized: with paths, the leaf directory name matters
        // more than the middle of the path.
        let tree = Rc::new(NavTree::new(
            SelectionMode::None,
            pango::EllipsizeMode::Middle,
            vec![("Watched Folders", render)],
        ));
        tree.set_category_visible(WATCHED_FOLDERS, true);

        let scrolled = ScrolledWindow::new();
        scrolled.set_min_content_height(120);
        scrolled.set_child(Some(tree.widget()));

        let on_remove: Rc<Callback<LibraryFolder>> = Rc::new(Callback::new());
        wire_remove_popover(&tree, &on_remove);

        Self {
            scrolled,
            tree,
            on_remove,
        }
    }

    /// Replaces the visible folders.
    pub fn set_folders(&self, folders: Vec<LibraryFolder>) {
        self.tree.set_items(WATCHED_FOLDERS, folders);
    }

    /// Registers the callback invoked with the folder whose remove action was
    /// chosen. Removal itself is the caller's job — this list only displays.
    pub fn connect_remove_requested<F: Fn(LibraryFolder) + 'static>(&self, f: F) {
        self.on_remove.set(f);
    }

    pub fn widget(&self) -> &ScrolledWindow {
        &self.scrolled
    }
}

/// Opens a "Remove folder" popover at the click point whenever the user
/// right-clicks a folder row.
fn wire_remove_popover(tree: &Rc<NavTree<LibraryFolder>>, on_remove: &Rc<Callback<LibraryFolder>>) {
    let view = tree.widget().clone();
    let on_remove = Rc::clone(on_remove);
    tree.connect_row_right_clicked(move |_category_id, folder, x, y| {
        let on_remove = Rc::clone(&on_remove);
        show_remove_popover(&view, x, y, move || on_remove.emit(folder.clone()));
    });
}
