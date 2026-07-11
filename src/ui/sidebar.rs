use std::cell::OnceCell;
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

struct SidebarState {
    genres: Vec<Genre>,
    artists: Vec<Artist>,
}

/// Genre / artist browser. Emits a `LibraryFilter` when the user selects an
/// entry.
#[derive(Clone)]
pub struct Sidebar {
    pub widget: GtkBox,
    inner: Rc<SidebarInner>,
}

struct SidebarInner {
    genres: ListBox,
    artists: ListBox,
    genres_section: Expander,
    artists_section: Expander,
    state: RefCell<SidebarState>,
    on_select: OnceCell<FilterCallback>,
}

impl Sidebar {
    pub fn new() -> Self {
        let all_btn = Button::with_label("Library");
        all_btn.add_css_class("flat");
        all_btn.set_margin_start(4);
        all_btn.set_margin_end(4);
        all_btn.set_margin_top(4);

        let genres = section_list();
        let artists = section_list();

        let inner = Rc::new(SidebarInner {
            genres: genres.clone(),
            artists: artists.clone(),
            genres_section: section("Genres", &genres, true),
            artists_section: section("Artists", &artists, false),
            state: RefCell::new(SidebarState {
                genres: Vec::new(),
                artists: Vec::new(),
            }),
            on_select: OnceCell::new(),
        });

        {
            let inner = Rc::clone(&inner);
            let lists = [genres.clone(), artists.clone()];
            all_btn.connect_clicked(move |_| {
                clear_selections(&lists);
                emit(&inner, LibraryFilter::All);
            });
        }
        {
            let inner = Rc::clone(&inner);
            let others = [artists.clone()];
            genres.connect_row_activated(move |_, row| {
                clear_selections(&others);
                let genre = inner
                    .state
                    .borrow()
                    .genres
                    .get(row.index() as usize)
                    .cloned();
                if let Some(genre) = genre {
                    emit(&inner, LibraryFilter::ByGenre(genre));
                }
            });
        }
        {
            let inner = Rc::clone(&inner);
            let others = [genres.clone()];
            artists.connect_row_activated(move |_, row| {
                clear_selections(&others);
                let artist = inner
                    .state
                    .borrow()
                    .artists
                    .get(row.index() as usize)
                    .cloned();
                if let Some(artist) = artist {
                    emit(&inner, LibraryFilter::ByArtist(artist));
                }
            });
        }

        let inner_box = GtkBox::new(Orientation::Vertical, 0);
        inner_box.append(&all_btn);
        inner_box.append(&inner.genres_section);
        inner_box.append(&inner.artists_section);

        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&inner_box));

        let widget = GtkBox::new(Orientation::Vertical, 0);
        widget.set_vexpand(true);
        widget.append(&scrolled);

        Self { widget, inner }
    }

    /// Registers the callback invoked whenever the user picks a filter.
    pub fn connect_filter_selected<F: Fn(LibraryFilter) + 'static>(&self, f: F) {
        let _ = self.inner.on_select.set(Rc::new(f));
    }

    /// Replaces every section's entries with the given distinct values.
    /// A section with no entries collapses itself, since there is nothing
    /// left inside it to expand into.
    pub fn populate(&self, genres: Vec<Genre>, artists: Vec<Artist>) {
        fill(&self.inner.genres, genres.iter().map(Genre::as_str));
        fill(&self.inner.artists, artists.iter().map(Artist::as_str));
        if genres.is_empty() {
            self.inner.genres_section.set_expanded(false);
        }
        if artists.is_empty() {
            self.inner.artists_section.set_expanded(false);
        }
        let mut state = self.inner.state.borrow_mut();
        state.genres = genres;
        state.artists = artists;
    }
}

/// Deselects every row in the given lists — keeps at most one section
/// selected across the whole sidebar.
fn clear_selections(lists: &[ListBox]) {
    for list in lists {
        list.unselect_all();
    }
}

fn emit(inner: &SidebarInner, filter: LibraryFilter) {
    if let Some(callback) = inner.on_select.get() {
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
