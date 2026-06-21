// Minimal HTTP server fixture. Exercises TcpBind (auto) and
// TcpAccept (best-effort) on the same receiver.
fn main() {
    let listener = std::net::TcpListener::bind("127.0.0.1:8080").unwrap();
    let (stream, _) = listener.accept().unwrap();
    let _ = stream;
    let _ = std::net::TcpStream::connect("127.0.0.1:9000");
}
