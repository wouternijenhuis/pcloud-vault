#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod replication;

fn main() {
    app::run();
}
