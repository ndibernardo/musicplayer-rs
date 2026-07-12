//! The app's widget vocabulary: small, typed building blocks shared by every
//! `ui/` component, so a visual idea is built (and styled) in exactly one
//! place.

mod buttons;
mod callback;
mod containers;
mod icons;
mod labels;
mod panel;
mod rows;
mod section;
mod value_list;

pub use buttons::flat_icon_button;
pub use buttons::flat_menu_button;
pub use callback::Callback;
pub use containers::remove_all_children;
pub use icons::AppIcon;
pub use labels::body_label;
pub use labels::caption_label;
pub use labels::numeric_dim_label;
pub use panel::SidebarPanel;
pub use rows::two_line_row;
pub use section::CollapsibleSection;
pub use value_list::ValueList;
