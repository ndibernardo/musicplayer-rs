use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::Expander;
use gtk4::Label;
use gtk4::ListBox;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::ScrolledWindow;
use gtk4::prelude::*;

use crate::library::filter::LibraryFilter;
use crate::library::track::Artist;
use crate::library::track::Genre;

type FilterCallback = Rc<dyn Fn(LibraryFilter)>;

/// Genre / artist browser. Emits a `LibraryFilter` when the user selects an
/// entry.
#[derive(Clone)]
pub struct Sidebar {
    pub widget: GtkBox,
    genres: ListBox,
    artists: ListBox,
    genre_values: Rc<RefCell<Vec<Genre>>>,
    artist_values: Rc<RefCell<Vec<Artist>>>,
    on_select: Rc<RefCell<Option<FilterCallback>>>,
}

impl Sidebar {
    pub fn new() -> Self {
        let on_select: Rc<RefCell<Option<FilterCallback>>> = Rc::new(RefCell::new(None));

        let all_btn = Button::with_label("All Tracks");
        all_btn.add_css_class("flat");
        all_btn.set_margin_start(4);
        all_btn.set_margin_end(4);
        all_btn.set_margin_top(4);

        let genres = section_list();
        let artists = section_list();

        {
            let on_select = Rc::clone(&on_select);
            let lists = [genres.clone(), artists.clone()];
            all_btn.connect_clicked(move |_| {
                clear_selections(&lists);
                emit(&on_select, LibraryFilter::All);
            });
        }

        let genre_values: Rc<RefCell<Vec<Genre>>> = Rc::new(RefCell::new(Vec::new()));
        let artist_values: Rc<RefCell<Vec<Artist>>> = Rc::new(RefCell::new(Vec::new()));

        {
            let on_select = Rc::clone(&on_select);
            let values = Rc::clone(&genre_values);
            let others = [artists.clone()];
            genres.connect_row_activated(move |_, row| {
                clear_selections(&others);
                if let Some(genre) = values.borrow().get(row.index() as usize).cloned() {
                    emit(&on_select, LibraryFilter::ByGenre(genre));
                }
            });
        }
        {
            let on_select = Rc::clone(&on_select);
            let values = Rc::clone(&artist_values);
            let others = [genres.clone()];
            artists.connect_row_activated(move |_, row| {
                clear_selections(&others);
                if let Some(artist) = values.borrow().get(row.index() as usize).cloned() {
                    emit(&on_select, LibraryFilter::ByArtist(artist));
                }
            });
        }

        let inner = GtkBox::new(Orientation::Vertical, 0);
        inner.append(&all_btn);
        inner.append(&section("Genres", &genres, true));
        inner.append(&section("Artists", &artists, false));

        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&inner));

        let widget = GtkBox::new(Orientation::Vertical, 0);
        widget.set_vexpand(true);
        widget.append(&scrolled);

        Self {
            widget,
            genres,
            artists,
            genre_values,
            artist_values,
            on_select,
        }
    }

    /// Registers the callback invoked whenever the user picks a filter.
    pub fn connect_filter_selected<F: Fn(LibraryFilter) + 'static>(&self, f: F) {
        *self.on_select.borrow_mut() = Some(Rc::new(f));
    }

    /// Replaces every section's entries with the given distinct values.
    pub fn populate(&self, genres: Vec<Genre>, artists: Vec<Artist>) {
        fill(&self.genres, genres.iter().map(Genre::as_str));
        fill(&self.artists, artists.iter().map(Artist::as_str));
        *self.genre_values.borrow_mut() = genres;
        *self.artist_values.borrow_mut() = artists;
    }
}

/// Deselects every row in the given lists — keeps at most one section
/// selected across the whole sidebar.
fn clear_selections(lists: &[ListBox]) {
    for list in lists {
        list.unselect_all();
    }
}

fn emit(on_select: &Rc<RefCell<Option<FilterCallback>>>, filter: LibraryFilter) {
    if let Some(callback) = on_select.borrow().as_ref() {
        callback(filter);
    }
}

fn section_list() -> ListBox {
    let list = ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    list.set_activate_on_single_click(true);
    list
}

fn section(title: &str, list: &ListBox, expanded: bool) -> Expander {
    let expander = Expander::new(Some(title));
    expander.set_expanded(expanded);
    expander.set_margin_start(4);
    expander.set_child(Some(list));
    expander
}

fn fill<'a>(list: &ListBox, entries: impl Iterator<Item = &'a str>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for entry in entries {
        let label = Label::new(Some(entry));
        label.set_xalign(0.0);
        label.set_margin_start(8);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        let row = ListBoxRow::new();
        row.set_child(Some(&label));
        list.append(&row);
    }
}
