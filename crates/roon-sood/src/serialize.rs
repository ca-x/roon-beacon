use std::collections::HashMap;

fn serialize_msg(msg_type: u8, props: &HashMap<String, Option<String>>) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);

    // Header
    buf.extend_from_slice(b"SOOD");
    buf.push(0x02);
    buf.push(msg_type);

    // TLV entries
    for (name, value) in props {
        let name_bytes = name.as_bytes();
        buf.push(name_bytes.len() as u8);
        buf.extend_from_slice(name_bytes);

        match value {
            None => {
                buf.push(0xFF);
                buf.push(0xFF);
            }
            Some(v) => {
                let v_bytes = v.as_bytes();
                let len = v_bytes.len() as u16;
                buf.push((len >> 8) as u8);
                buf.push((len & 0xFF) as u8);
                buf.extend_from_slice(v_bytes);
            }
        }
    }

    buf
}

/// Serialize a SOOD query message from a map of properties.
pub fn serialize_query(props: &HashMap<String, Option<String>>) -> Vec<u8> {
    serialize_msg(b'Q', props)
}

/// Serialize a SOOD response message from a map of properties.
pub fn serialize_response(props: &HashMap<String, Option<String>>) -> Vec<u8> {
    serialize_msg(b'R', props)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SoodType;
    use crate::parse::parse;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    fn test_addr() -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 9003))
    }

    #[test]
    fn test_serialize_empty_props() {
        let props = HashMap::new();
        let buf = serialize_query(&props);
        assert_eq!(&buf[0..4], b"SOOD");
        assert_eq!(buf[4], 0x02);
        assert_eq!(buf[5], b'Q');
        assert_eq!(buf.len(), 6);
    }

    #[test]
    fn test_serialize_response_empty() {
        let props = HashMap::new();
        let buf = serialize_response(&props);
        assert_eq!(&buf[0..4], b"SOOD");
        assert_eq!(buf[4], 0x02);
        assert_eq!(buf[5], b'R');
        assert_eq!(buf.len(), 6);
    }

    #[test]
    fn test_serialize_with_value() {
        let mut props = HashMap::new();
        props.insert("key".to_string(), Some("value".to_string()));
        let buf = serialize_query(&props);

        assert_eq!(&buf[0..6], b"SOOD\x02Q");
        assert_eq!(buf[6], 3);
        assert_eq!(&buf[7..10], b"key");
        assert_eq!(buf[10], 0x00);
        assert_eq!(buf[11], 0x05);
        assert_eq!(&buf[12..17], b"value");
    }

    #[test]
    fn test_serialize_null_value() {
        let mut props = HashMap::new();
        props.insert("key".to_string(), None);
        let buf = serialize_query(&props);

        assert_eq!(buf[6], 3);
        assert_eq!(&buf[7..10], b"key");
        assert_eq!(buf[10], 0xFF);
        assert_eq!(buf[11], 0xFF);
        assert_eq!(buf.len(), 12);
    }

    #[test]
    fn test_serialize_empty_value() {
        let mut props = HashMap::new();
        props.insert("k".to_string(), Some(String::new()));
        let buf = serialize_query(&props);

        assert_eq!(buf[6], 1);
        assert_eq!(buf[7], b'k');
        assert_eq!(buf[8], 0x00);
        assert_eq!(buf[9], 0x00);
        assert_eq!(buf.len(), 10);
    }

    #[test]
    fn test_roundtrip_single_property() {
        let mut props = HashMap::new();
        props.insert("service".to_string(), Some("roon-core".to_string()));

        let buf = serialize_query(&props);
        let msg = parse(&buf, test_addr()).unwrap();

        assert_eq!(msg.msg_type, SoodType::Query);
        assert_eq!(
            msg.props.get("service"),
            Some(&Some("roon-core".to_string()))
        );
    }

    #[test]
    fn test_roundtrip_response() {
        let mut props = HashMap::new();
        props.insert("_tid".to_string(), Some("abc123".to_string()));
        props.insert("http_port".to_string(), Some("9330".to_string()));

        let buf = serialize_response(&props);
        let msg = parse(&buf, test_addr()).unwrap();

        assert_eq!(msg.msg_type, SoodType::Response);
        assert_eq!(msg.props.get("_tid"), Some(&Some("abc123".to_string())));
        assert_eq!(msg.props.get("http_port"), Some(&Some("9330".to_string())));
    }

    #[test]
    fn test_roundtrip_null_value() {
        let mut props = HashMap::new();
        props.insert("tid".to_string(), None);

        let buf = serialize_query(&props);
        let msg = parse(&buf, test_addr()).unwrap();

        assert_eq!(msg.props.get("tid"), Some(&None));
    }

    #[test]
    fn test_roundtrip_multiple_properties() {
        let mut props = HashMap::new();
        props.insert("a".to_string(), Some("hello".to_string()));
        props.insert("b".to_string(), None);
        props.insert("c".to_string(), Some(String::new()));

        let buf = serialize_query(&props);
        let msg = parse(&buf, test_addr()).unwrap();

        assert_eq!(msg.props.len(), 3);
        assert_eq!(msg.props.get("a"), Some(&Some("hello".to_string())));
        assert_eq!(msg.props.get("b"), Some(&None));
        assert_eq!(msg.props.get("c"), Some(&Some(String::new())));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::SoodType;
    use crate::parse::parse;
    use proptest::prelude::*;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    fn test_addr() -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 9003))
    }

    fn prop_name() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9]{1,30}".prop_filter("must not be reserved", |s| {
            s != "_replyaddr" && s != "_replyport"
        })
    }

    fn prop_value() -> impl Strategy<Value = Option<String>> {
        prop_oneof![
            Just(None),
            Just(Some(String::new())),
            "[a-zA-Z0-9 ._\\-]{1,200}".prop_map(Some),
        ]
    }

    proptest! {
        #[test]
        fn roundtrip(
            entries in proptest::collection::hash_map(prop_name(), prop_value(), 0..10)
        ) {
            let buf = serialize_query(&entries);
            let msg = parse(&buf, test_addr()).unwrap();

            prop_assert_eq!(msg.msg_type, SoodType::Query);
            prop_assert_eq!(msg.props.len(), entries.len());

            for (key, value) in &entries {
                prop_assert_eq!(msg.props.get(key), Some(value));
            }
        }
    }
}
