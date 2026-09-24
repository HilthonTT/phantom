use super::destination::{Destination, PortString, add_port_to_hostname, get_ip_with_port};

#[test]
fn ips_get_default_ports() {
    assert_eq!(
        get_ip_with_port("1.1.1.1"),
        Some(Destination::Literal(
            "1.1.1.1:8448".parse().expect("valid addr")
        ))
    );
    assert_eq!(
        get_ip_with_port("dead:beef::"),
        Some(Destination::Literal(
            "[dead:beef::]:8448".parse().expect("valid addr")
        ))
    );
}

#[test]
fn ips_keep_custom_ports() {
    assert_eq!(
        get_ip_with_port("1.1.1.1:1234"),
        Some(Destination::Literal(
            "1.1.1.1:1234".parse().expect("valid addr")
        ))
    );
    assert_eq!(
        get_ip_with_port("[dead::beef]:8933"),
        Some(Destination::Literal(
            "[dead::beef]:8933".parse().expect("valid addr")
        ))
    );
}

#[test]
fn a_hostname_is_not_an_address() {
    assert_eq!(get_ip_with_port("example.com"), None);
    assert_eq!(get_ip_with_port("example.com:1337"), None);
}

#[test]
fn hostnames_get_default_ports() {
    assert_eq!(
        add_port_to_hostname("example.com"),
        Destination::Named(String::from("example.com"), Destination::default_port())
    );
}

#[test]
fn hostnames_keep_custom_ports() {
    assert_eq!(
        add_port_to_hostname("example.com:1337"),
        Destination::Named(
            String::from("example.com"),
            PortString::from(":1337").expect("fits")
        )
    );
}

#[test]
fn the_port_is_read_back_off_a_destination() {
    assert_eq!(add_port_to_hostname("example.com").port(), Some(8448));
    assert_eq!(add_port_to_hostname("example.com:1337").port(), Some(1337));
    assert_eq!(
        get_ip_with_port("1.1.1.1:1234").and_then(|d| d.port()),
        Some(1234)
    );
}

#[test]
fn a_malformed_port_reads_as_none_rather_than_panicking() {
    let dest = Destination::Named(
        String::from("example.com"),
        PortString::from("").expect("fits"),
    );

    assert_eq!(dest.port(), None);
}

#[test]
fn a_destination_renders_as_a_url_and_as_an_authority() {
    let named = add_port_to_hostname("example.com:1337");
    assert_eq!(named.https_string(), "https://example.com:1337");
    assert_eq!(named.uri_string(), "example.com:1337");
    assert_eq!(named.hostname(), "example.com");

    let literal = get_ip_with_port("1.1.1.1:1234").expect("an address");
    assert_eq!(literal.https_string(), "https://1.1.1.1:1234");
    assert_eq!(literal.uri_string(), "1.1.1.1:1234");
    assert_eq!(literal.hostname(), "1.1.1.1");
}
