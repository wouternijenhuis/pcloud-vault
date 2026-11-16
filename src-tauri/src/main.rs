#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod features;
mod folders;
mod jobs;
mod replication;
mod transfers;

fn main() {
    app::run();
}
