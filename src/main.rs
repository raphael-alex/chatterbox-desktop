mod app;

use app::App;
use dioxus::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    dioxus::launch(App);
}
