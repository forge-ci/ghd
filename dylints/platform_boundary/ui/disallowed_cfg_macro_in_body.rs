// A cfg! in a private function body was invisible to an item-attribute-only
// lint; this proves the pre-expansion scan also sees statement position.
fn cfg_macro_in_private_body() -> u8 {
    if cfg!(unix) { 1 } else { 2 }
}

fn main() {}
