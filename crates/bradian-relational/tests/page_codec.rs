use bradian_relational::page::{
    Page, PageError, PageKind, MAX_PAYLOAD_SIZE, PAGE_HEADER_SIZE, PAGE_SIZE, PAGE_VERSION,
};

#[test]
fn physical_page_roundtrip_for_all_kinds() {
    for kind in [PageKind::Data, PageKind::Index, PageKind::FreeMap, PageKind::Metadata] {
        for len in [0, 1, 256, MAX_PAYLOAD_SIZE] {
            let payload = (0..len).map(|i| (i % 251) as u8).collect();
            let original = Page::new(42, 8192, kind, payload).unwrap();
            let encoded = original.encode().unwrap();
            assert_eq!(encoded.len(), PAGE_SIZE);
            assert_eq!(Page::decode(&encoded).unwrap(), original);
            assert_eq!(&encoded[0..4], b"BRPG");
            assert_eq!(u16::from_le_bytes([encoded[4], encoded[5]]), PAGE_VERSION);
            assert_eq!(PAGE_HEADER_SIZE, 32);
        }
    }
}

#[test]
fn rejects_truncated_or_oversized_pages_and_payloads() {
    let valid = Page::new(1, 0, PageKind::Data, vec![7; 8]).unwrap().encode().unwrap();
    assert_eq!(Page::decode(&valid[..PAGE_SIZE - 1]), Err(PageError::InvalidLength(PAGE_SIZE - 1)));
    let mut larger = valid.to_vec();
    larger.push(0);
    assert_eq!(Page::decode(&larger), Err(PageError::InvalidLength(PAGE_SIZE + 1)));
    assert_eq!(
        Page::new(1, 0, PageKind::Data, vec![0; MAX_PAYLOAD_SIZE + 1]),
        Err(PageError::InvalidPayloadLength(MAX_PAYLOAD_SIZE + 1))
    );
}

#[test]
fn rejects_page_corruption() {
    let valid = Page::new(7, 10, PageKind::Data, vec![1, 2, 3]).unwrap().encode().unwrap();
    for i in [8, 18, 32, 34, 128, 4095] {
        let mut corrupt = valid;
        corrupt[i] ^= 0x7f;
        assert_eq!(Page::decode(&corrupt), Err(PageError::ChecksumMismatch), "byte {i}");
    }
}

#[test]
fn rejects_unknown_format_versions_types_and_flags() {
    let valid = Page::new(99, 77, PageKind::Index, vec![8]).unwrap().encode().unwrap();
    let mut bad_magic = valid;
    bad_magic[0] = 0;
    assert_eq!(Page::decode(&bad_magic), Err(PageError::InvalidMagic));

    // ADR 0064 adds version 2 as a valid on-disk format (BRPG v2). The
    // next unknown version is 3; that's what this fail-closed case now
    // asserts against.
    let mut future_version = valid;
    future_version[4..6].copy_from_slice(&3_u16.to_le_bytes());
    assert_eq!(Page::decode(&future_version), Err(PageError::UnsupportedVersion(3)));

    let mut unknown_type = valid;
    unknown_type[6] = 255;
    assert_eq!(Page::decode(&unknown_type), Err(PageError::UnknownKind(255)));

    let mut flags = valid;
    flags[7] = 1;
    assert_eq!(Page::decode(&flags), Err(PageError::UnsupportedFlags(1)));

    let mut oversized = valid;
    oversized[24..28].copy_from_slice(&(PAGE_SIZE as u32).to_le_bytes());
    assert_eq!(Page::decode(&oversized), Err(PageError::InvalidPayloadLength(PAGE_SIZE)));
}

#[test]
fn little_endian_binary_layout_is_defined() {
    let encoded = Page::new(
        0x0102_0304_0506_0708,
        0x1112_1314_1516_1718,
        PageKind::Data,
        vec![0xab, 0xcd],
    ).unwrap().encode().unwrap();
    assert_eq!(&encoded[8..16], &[8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(&encoded[16..24], &[0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11]);
    assert_eq!(&encoded[24..28], &[2, 0, 0, 0]);
    assert_eq!(&encoded[32..34], &[0xab, 0xcd]);
    assert!(encoded[34..].iter().all(|v| *v == 0));
}

#[test]
fn checksum_is_not_confused_with_authentication() {
    // The checksum catches accidental corruption, but attackers can recompute
    // CRCs. Encryption/authentication is explicitly outside this codec.
    let encoded = Page::new(1, 0, PageKind::Data, vec![42]).unwrap().encode().unwrap();
    assert_eq!(Page::decode(&encoded).unwrap().payload, vec![42]);
}
