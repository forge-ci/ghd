// Native platform paths are denied outside ghd-platform's selector
// file and its platforms/** trees, even when nothing else in the file
// selects a host with cfg.
fn touches_native_path() {
    let windows_sys = ();
}

fn main() {}
