/// Which content view the main window is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Grid,
}

impl ViewMode {
    /// The `gtk::Stack` child name this mode maps to. Also the persisted value.
    pub fn child_name(self) -> &'static str {
        match self {
            ViewMode::List => "list",
            ViewMode::Grid => "grid",
        }
    }

    /// Parses a persisted `child_name` back into a mode, or `None` if unknown.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "list" => Some(ViewMode::List),
            "grid" => Some(ViewMode::Grid),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_maps_to_list_child() {
        assert_eq!(ViewMode::List.child_name(), "list");
    }

    #[test]
    fn grid_maps_to_grid_child() {
        assert_eq!(ViewMode::Grid.child_name(), "grid");
    }

    #[test]
    fn from_name_round_trips_each_mode() {
        assert_eq!(ViewMode::from_name("list"), Some(ViewMode::List));
        assert_eq!(ViewMode::from_name("grid"), Some(ViewMode::Grid));
    }

    #[test]
    fn from_name_rejects_unknown_value() {
        assert_eq!(ViewMode::from_name("carousel"), None);
    }
}
