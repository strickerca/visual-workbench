use super::*;
#[test]
fn legacy_refusal_serialized_bytes_remain_exact() -> Result<()> {
    let p = Packet {
        header: Header::Refused {
            sequence: 2,
            error: Error::PartialInput,
            finite_attempt: None,
        },
        payload: vec![],
    };
    let json = serde_json::to_vec(&p.header).map_err(|_| Error::Invalid)?;
    assert_eq!(
        json,
        br#"{"reply":"Refused","sequence":2,"error":"PartialInput"}"#
    );
    Ok(())
}
#[test]
fn actual_pending_packet_roundtrip_preserves_count_qpc_and_unknown_flag() -> Result<()> {
    let p = Packet {
        header: Header::Refused {
            sequence: 2,
            error: Error::PartialInput,
            finite_attempt: Some(FiniteAttempt {
                expected_count: 3,
                accepted_count: 2,
                accepted_qpc_100ns: Some(99),
                error: Some(Error::PartialInput),
                retirement: FiniteRetirement::Pending {
                    held_count: 1,
                    uncertain: false,
                    error: Error::TargetChanged,
                },
            }),
        },
        payload: vec![],
    };
    let mut bytes = vec![];
    write_packet(&mut bytes, &p)?;
    let read = read_packet(&mut std::io::Cursor::new(bytes))?;
    assert_eq!(
        serde_json::to_value(read.header).map_err(|_| Error::Invalid)?,
        serde_json::to_value(p.header).map_err(|_| Error::Invalid)?
    );
    assert!(read.payload.is_empty());
    Ok(())
}
#[test]
fn unknown_retirement_or_extra_global_release_claim_refused() {
    assert!(serde_json::from_str::<FiniteRetirement>(r#"{"GlobalReleased":{}}"#).is_err());
    assert!(serde_json::from_str::<FiniteAttempt>(r#"{"expected_count":3,"accepted_count":2,"accepted_qpc_100ns":99,"error":"PartialInput","retirement":"Complete","physical_release":true}"#).is_err())
}
