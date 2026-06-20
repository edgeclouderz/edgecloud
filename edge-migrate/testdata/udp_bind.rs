// UDP bind fixture. Exercises UdpBind (auto).
fn main() {
    let sock = std::net::UdpSocket::bind("0.0.0.0:5353").unwrap();
    let _ = sock;
}
