/// Every symbolic icon the app uses. `Button::from_icon_name("lst-add-symbolic")`
/// typo-compiles and renders a broken image; `AppIcon::ListAdd.name()` doesn't.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    ListAdd,
    ListDragHandle,
    EditClear,
    SidebarShow,
    SidebarShowRight,
    FolderNew,
    ViewRefresh,
    ViewList,
    ViewGrid,
    ViewSortAscending,
    ViewSortDescending,
    MediaSkipBackward,
    MediaPlaybackStart,
    MediaPlaybackPause,
    MediaPlaybackStop,
    MediaSkipForward,
    AudioVolumeMedium,
    OpenMenu,
}

impl AppIcon {
    /// The freedesktop icon name.
    pub const fn name(self) -> &'static str {
        match self {
            AppIcon::ListAdd => "list-add-symbolic",
            AppIcon::ListDragHandle => "list-drag-handle-symbolic",
            AppIcon::EditClear => "edit-clear-symbolic",
            AppIcon::SidebarShow => "sidebar-show-symbolic",
            AppIcon::SidebarShowRight => "sidebar-show-right-symbolic",
            AppIcon::FolderNew => "folder-new-symbolic",
            AppIcon::ViewRefresh => "view-refresh-symbolic",
            AppIcon::ViewList => "view-list-symbolic",
            AppIcon::ViewGrid => "view-grid-symbolic",
            AppIcon::ViewSortAscending => "view-sort-ascending-symbolic",
            AppIcon::ViewSortDescending => "view-sort-descending-symbolic",
            AppIcon::MediaSkipBackward => "media-skip-backward-symbolic",
            AppIcon::MediaPlaybackStart => "media-playback-start-symbolic",
            AppIcon::MediaPlaybackPause => "media-playback-pause-symbolic",
            AppIcon::MediaPlaybackStop => "media-playback-stop-symbolic",
            AppIcon::MediaSkipForward => "media-skip-forward-symbolic",
            AppIcon::AudioVolumeMedium => "audio-volume-medium-symbolic",
            AppIcon::OpenMenu => "open-menu-symbolic",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ICONS: [AppIcon; 18] = [
        AppIcon::ListAdd,
        AppIcon::ListDragHandle,
        AppIcon::EditClear,
        AppIcon::SidebarShow,
        AppIcon::SidebarShowRight,
        AppIcon::FolderNew,
        AppIcon::ViewRefresh,
        AppIcon::ViewList,
        AppIcon::ViewGrid,
        AppIcon::ViewSortAscending,
        AppIcon::ViewSortDescending,
        AppIcon::MediaSkipBackward,
        AppIcon::MediaPlaybackStart,
        AppIcon::MediaPlaybackPause,
        AppIcon::MediaPlaybackStop,
        AppIcon::MediaSkipForward,
        AppIcon::AudioVolumeMedium,
        AppIcon::OpenMenu,
    ];

    #[test]
    fn icon_names_are_unique() {
        let mut names: Vec<&str> = ALL_ICONS.iter().map(|i| i.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ALL_ICONS.len());
    }

    #[test]
    fn every_icon_is_symbolic() {
        for icon in ALL_ICONS {
            assert!(
                icon.name().ends_with("-symbolic"),
                "{} is not a symbolic icon name",
                icon.name()
            );
        }
    }
}
