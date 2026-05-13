use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use crate::{SoodError, SoodMessage, SoodType};

/// Parse a raw SOOD datagram into a `SoodMessage`.
///
/// The protocol format is:
///   - 4 bytes magic: b"SOOD"
///   - 1 byte version: 0x02
///   - 1 byte type: b'Q' for Query, b'R' for Response
///   - Repeated TLV entries until end of buffer:
///     - 1 byte: name length (must be > 0)
///     - N bytes: name (UTF-8)
///     - 2 bytes: value length (big-endian); 0xFFFF means null
///     - M bytes: value (UTF-8), absent when null
///
/// Special properties `_replyaddr` and `_replyport` override the
/// source address and are removed from the returned props.
pub fn parse(buf: &[u8], from: SocketAddr) -> Result<SoodMessage, SoodError> {
    if buf.len() < 6 {
        return Err(SoodError::BufferTooShort(buf.len()));
    }

    if &buf[0..4] != b"SOOD" {
        return Err(SoodError::InvalidMagic);
    }

    if buf[4] != 0x02 {
        return Err(SoodError::UnsupportedVersion(buf[4]));
    }

    let msg_type = match buf[5] {
        b'Q' => SoodType::Query,
        b'R' => SoodType::Response,
        other => return Err(SoodError::InvalidType(other)),
    };

    let mut props = HashMap::new();
    let mut pos = 6;

    while pos < buf.len() {
        // Read name length
        let name_len = buf[pos] as usize;
        pos += 1;

        if name_len == 0 {
            return Err(SoodError::ZeroLengthName);
        }

        // Read name
        let remaining = buf.len() - pos;
        if name_len > remaining {
            return Err(SoodError::TruncatedName {
                name_len,
                remaining,
            });
        }
        let name = String::from_utf8(buf[pos..pos + name_len].to_vec())?;
        pos += name_len;

        // Read value length (2 bytes big-endian)
        let remaining = buf.len() - pos;
        if remaining < 2 {
            return Err(SoodError::TruncatedValueHeader { remaining });
        }
        let value_len = ((buf[pos] as u16) << 8) | (buf[pos + 1] as u16);
        pos += 2;

        let val = if value_len == 0xFFFF {
            None
        } else if value_len == 0 {
            Some(String::new())
        } else {
            let vlen = value_len as usize;
            let remaining = buf.len() - pos;
            if vlen > remaining {
                return Err(SoodError::TruncatedValue {
                    value_len: vlen,
                    remaining,
                });
            }
            let v = std::str::from_utf8(&buf[pos..pos + vlen])
                .map_err(|_| SoodError::InvalidValueUtf8)?;
            pos += vlen;
            Some(v.to_string())
        };

        props.insert(name, val);
    }

    // Post-process: override from address with _replyaddr/_replyport
    let mut result_addr = from;

    if let Some(Some(addr_str)) = props.remove("_replyaddr")
        && let Ok(ip) = addr_str.parse::<IpAddr>()
    {
        result_addr.set_ip(ip);
    }

    if let Some(Some(port_str)) = props.remove("_replyport")
        && let Ok(port) = port_str.parse::<u16>()
    {
        result_addr.set_port(port);
    }

    Ok(SoodMessage {
        from: result_addr,
        msg_type,
        props,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};

    fn test_addr() -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 100), 9003))
    }

    #[test]
    fn test_parse_valid_query() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'Q');
        // Property: "query_service_id" = "test-service"
        let name = b"svc";
        let value = b"test-service";
        buf.push(name.len() as u8);
        buf.extend_from_slice(name);
        buf.push((value.len() >> 8) as u8);
        buf.push((value.len() & 0xFF) as u8);
        buf.extend_from_slice(value);

        let msg = parse(&buf, test_addr()).unwrap();
        assert_eq!(msg.msg_type, SoodType::Query);
        assert_eq!(
            msg.props.get("svc"),
            Some(&Some("test-service".to_string()))
        );
        assert_eq!(msg.from, test_addr());
    }

    #[test]
    fn test_parse_valid_response() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'R');

        let msg = parse(&buf, test_addr()).unwrap();
        assert_eq!(msg.msg_type, SoodType::Response);
        assert!(msg.props.is_empty());
    }

    #[test]
    fn test_parse_null_value() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'Q');
        let name = b"key";
        buf.push(name.len() as u8);
        buf.extend_from_slice(name);
        buf.push(0xFF);
        buf.push(0xFF);

        let msg = parse(&buf, test_addr()).unwrap();
        assert_eq!(msg.props.get("key"), Some(&None));
    }

    #[test]
    fn test_parse_empty_value() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'Q');
        let name = b"key";
        buf.push(name.len() as u8);
        buf.extend_from_slice(name);
        buf.push(0x00);
        buf.push(0x00);

        let msg = parse(&buf, test_addr()).unwrap();
        assert_eq!(msg.props.get("key"), Some(&Some(String::new())));
    }

    #[test]
    fn test_parse_replyaddr_override() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'R');
        // _replyaddr = "10.0.0.1"
        let name = b"_replyaddr";
        let value = b"10.0.0.1";
        buf.push(name.len() as u8);
        buf.extend_from_slice(name);
        buf.push((value.len() >> 8) as u8);
        buf.push((value.len() & 0xFF) as u8);
        buf.extend_from_slice(value);
        // _replyport = "5555"
        let name = b"_replyport";
        let value = b"5555";
        buf.push(name.len() as u8);
        buf.extend_from_slice(name);
        buf.push((value.len() >> 8) as u8);
        buf.push((value.len() & 0xFF) as u8);
        buf.extend_from_slice(value);

        let msg = parse(&buf, test_addr()).unwrap();
        assert_eq!(
            msg.from,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 5555))
        );
        // _replyaddr and _replyport should be removed from props
        assert!(!msg.props.contains_key("_replyaddr"));
        assert!(!msg.props.contains_key("_replyport"));
    }

    #[test]
    fn test_parse_multiple_properties() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'Q');

        for (name, value) in &[("a", "hello"), ("b", "world")] {
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.push((value.len() >> 8) as u8);
            buf.push((value.len() & 0xFF) as u8);
            buf.extend_from_slice(value.as_bytes());
        }

        let msg = parse(&buf, test_addr()).unwrap();
        assert_eq!(msg.props.len(), 2);
        assert_eq!(msg.props.get("a"), Some(&Some("hello".to_string())));
        assert_eq!(msg.props.get("b"), Some(&Some("world".to_string())));
    }

    #[test]
    fn test_error_buffer_too_short() {
        let buf = b"SOO";
        assert!(matches!(
            parse(buf, test_addr()),
            Err(SoodError::BufferTooShort(3))
        ));
    }

    #[test]
    fn test_error_invalid_magic() {
        let buf = b"FOOD\x02Q";
        assert!(matches!(
            parse(buf, test_addr()),
            Err(SoodError::InvalidMagic)
        ));
    }

    #[test]
    fn test_error_unsupported_version() {
        let buf = b"SOOD\x03Q";
        assert!(matches!(
            parse(buf, test_addr()),
            Err(SoodError::UnsupportedVersion(3))
        ));
    }

    #[test]
    fn test_error_zero_length_name() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'Q');
        buf.push(0x00); // zero-length name

        assert!(matches!(
            parse(&buf, test_addr()),
            Err(SoodError::ZeroLengthName)
        ));
    }

    #[test]
    fn test_error_truncated_name() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'Q');
        buf.push(0x05); // name_len = 5
        buf.extend_from_slice(b"ab"); // only 2 bytes

        assert!(matches!(
            parse(&buf, test_addr()),
            Err(SoodError::TruncatedName {
                name_len: 5,
                remaining: 2,
            })
        ));
    }

    #[test]
    fn test_error_truncated_value_header() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'Q');
        buf.push(0x01); // name_len = 1
        buf.push(b'x'); // name
        buf.push(0x00); // only 1 byte of value length header

        assert!(matches!(
            parse(&buf, test_addr()),
            Err(SoodError::TruncatedValueHeader { remaining: 1 })
        ));
    }

    #[test]
    fn test_error_truncated_value() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD");
        buf.push(0x02);
        buf.push(b'Q');
        buf.push(0x01); // name_len = 1
        buf.push(b'x'); // name
        buf.push(0x00);
        buf.push(0x05); // value_len = 5
        buf.extend_from_slice(b"ab"); // only 2 bytes

        assert!(matches!(
            parse(&buf, test_addr()),
            Err(SoodError::TruncatedValue {
                value_len: 5,
                remaining: 2,
            })
        ));
    }
}
