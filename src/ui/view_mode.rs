/// Which content view the main window is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Grid,
}

impl ViewMode {
    /// The `gtk::Stack` child name this mode maps to.
    pub fn child_name(self) -> &'static str {
        match self {
            ViewMode::List => "list",
            ViewMode::Grid => "grid",
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
}
