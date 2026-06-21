// fs open/read fixture. Exercises FsOpen (auto).
fn main() {
    let _ = std::fs::File::open("data.txt");
    let _ = std::fs::read_to_string("data.txt");
    let _ = std::fs::write("out.txt", b"hello");
}
