mod adapters;
mod domain;
#[cfg(feature = "ui")]
mod ui;

fn main() {
    #[cfg(feature = "ui")]
    ui::run();
}
