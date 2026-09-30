#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod platform;

fn main() {
    platform::configure_renderer();
    work_token_monitor_lib::run();
}
