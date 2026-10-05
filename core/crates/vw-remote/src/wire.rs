//! Private bounded pipes. No paths, caller ACKs or executable hashes in commands.
use crate::*;
use std::io::{Read, Write};
const HEADER_LIMIT: usize = 64 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", deny_unknown_fields)]
pub enum Command {
    OpenVideo {
        sequence: u64,
        target: Target,
        scope: Scope,
        owner_pid: u32,
    },
    Poll {
        sequence: u64,
        force_keyframe: bool,
    },
    OpenInput {
        sequence: u64,
        target: Target,
        binding: Binding,
        owner_pid: u32,
        profile: Option<profile::PackagedProfile>,
    },
    Pen {
        sequence: u64,
        input_seq: u64,
        sample: PenSample,
    },
    Finite {
        sequence: u64,
        input_seq: u64,
        action: FiniteAction,
    },
    RefreshAuthority {
        sequence: u64,
        profile_digest: String,
    },
    Check {
        sequence: u64,
    },
    Stop {
        sequence: u64,
    },
}
impl Command {
    pub fn sequence(&self) -> u64 {
        match self {
            Self::OpenVideo { sequence, .. }
            | Self::Poll { sequence, .. }
            | Self::OpenInput { sequence, .. }
            | Self::Pen { sequence, .. }
            | Self::Finite { sequence, .. }
            | Self::RefreshAuthority { sequence, .. }
            | Self::Check { sequence }
            | Self::Stop { sequence } => *sequence,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Phase {
    Hover,
    Down,
    Move,
    Up,
    Leave,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PenSample {
    pub phase: Phase,
    pub x: i32,
    pub y: i32,
    pub pressure: u32,
    pub tilt_x: i32,
    pub tilt_y: i32,
    pub rotation: u32,
    pub pen_flags: u32,
}
impl PenSample {
    pub fn validate(&self) -> Result<()> {
        if self.pressure > 1024
            || self.rotation > 359
            || !(-90..=90).contains(&self.tilt_x)
            || !(-90..=90).contains(&self.tilt_y)
            || self.pen_flags & !7 != 0
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FiniteAction {
    Click {
        x: i32,
        y: i32,
        button: u8,
    },
    Wheel {
        x: i32,
        y: i32,
        delta: i32,
    },
    Shortcut {
        profile_digest: String,
        action: u32,
        focus_runtime_hash: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum FiniteRetirement {
    Complete,
    Pending {
        held_count: u32,
        uncertain: bool,
        error: Error,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiniteAttempt {
    pub expected_count: u32,
    pub accepted_count: u32,
    pub accepted_qpc_100ns: Option<u64>,
    pub error: Option<Error>,
    pub retirement: FiniteRetirement,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PenReleaseWitness {
    pub binding: Binding,
    pub held_before_release: bool,
    pub release_started_qpc: u64,
    pub release_completed_qpc: u64,
    pub qpc_frequency: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "reply", deny_unknown_fields)]
pub enum Header {
    Ready {
        sequence: u64,
        capabilities: Option<EncoderCapabilities>,
        input_authority: Option<profile::Authority>,
    },
    Video {
        sequence: u64,
        scope: Scope,
        config_generation: u64,
        frame_id: u64,
        pts_100ns: i64,
        captured_qpc_100ns: u64,
        keyframe: bool,
        config: Option<VideoConfig>,
        payload_bytes: u32,
    },
    Injected {
        sequence: u64,
        binding: Binding,
        input_seq: u64,
        accepted_qpc_100ns: u64,
    },
    Authority {
        sequence: u64,
        binding: Binding,
        authority: Option<profile::Authority>,
    },
    Idle {
        sequence: u64,
    },
    Stopped {
        sequence: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pen_release: Option<PenReleaseWitness>,
    },
    Refused {
        sequence: u64,
        error: Error,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finite_attempt: Option<FiniteAttempt>,
    },
}
impl Header {
    pub fn sequence(&self) -> u64 {
        match self {
            Self::Ready { sequence, .. }
            | Self::Video { sequence, .. }
            | Self::Injected { sequence, .. }
            | Self::Authority { sequence, .. }
            | Self::Idle { sequence }
            | Self::Stopped { sequence, .. }
            | Self::Refused { sequence, .. } => *sequence,
        }
    }
}
pub struct Packet {
    pub header: Header,
    pub payload: Vec<u8>,
}
pub fn read_command(r: &mut impl Read) -> Result<Command> {
    let bytes = read_bounded(r, HEADER_LIMIT)?;
    let v: Command = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    if v.sequence() == 0 {
        return Err(Error::Invalid);
    }
    Ok(v)
}
pub fn write_command(w: &mut impl Write, v: &Command) -> Result<()> {
    write_bounded(
        w,
        &serde_json::to_vec(v).map_err(|_| Error::Invalid)?,
        HEADER_LIMIT,
    )
}
pub fn read_packet(r: &mut impl Read) -> Result<Packet> {
    let bytes = read_bounded(r, HEADER_LIMIT)?;
    let header: Header = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    let payload = read_bounded(r, MAX_ACCESS_UNIT)?;
    let expected = if let Header::Video { payload_bytes, .. } = &header {
        *payload_bytes as usize
    } else {
        0
    };
    if header.sequence() == 0 || expected != payload.len() {
        return Err(Error::Invalid);
    }
    Ok(Packet { header, payload })
}
pub fn write_packet(w: &mut impl Write, p: &Packet) -> Result<()> {
    let expected = if let Header::Video { payload_bytes, .. } = &p.header {
        *payload_bytes as usize
    } else {
        0
    };
    if p.payload.len() != expected {
        return Err(Error::Invalid);
    }
    write_bounded(
        w,
        &serde_json::to_vec(&p.header).map_err(|_| Error::Invalid)?,
        HEADER_LIMIT,
    )?;
    write_bounded(w, &p.payload, MAX_ACCESS_UNIT)
}
fn read_bounded(r: &mut impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut size = [0; 4];
    r.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    if size > limit {
        return Err(Error::Limit);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(size).map_err(|_| Error::Limit)?;
    out.resize(size, 0);
    r.read_exact(&mut out)?;
    Ok(out)
}
fn write_bounded(w: &mut impl Write, bytes: &[u8], limit: usize) -> Result<()> {
    if bytes.len() > limit {
        return Err(Error::Limit);
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(bytes)?;
    w.flush()?;
    Ok(())
}

#[cfg(test)]
#[path = "wire/finite_tests.rs"]
mod finite_tests;

#[cfg(test)]
mod refresh_tests {
    use super::*;
    #[test]
    fn refresh_command_roundtrip_is_distinct_from_check() -> Result<()> {
        let command = Command::RefreshAuthority {
            sequence: 9,
            profile_digest: "a".repeat(64),
        };
        let mut bytes = Vec::new();
        write_command(&mut bytes, &command)?;
        assert!(matches!(
            read_command(&mut bytes.as_slice())?,
            Command::RefreshAuthority { sequence: 9, .. }
        ));
        Ok(())
    }
    #[test]
    fn absent_authority_is_a_bound_nonfatal_payload_free_response() -> Result<()> {
        let binding = Binding {
            scope: Scope {
                connection_epoch: 1,
                capture_session_id: "018bcfe5-6800-7000-8000-000000000001".into(),
                source_generation: 1,
                target_token: "018bcfe5-6800-7000-8000-000000000002".into(),
                geometry_revision: 1,
            },
            input_session_id: "018bcfe5-6800-7000-8000-000000000003".into(),
        };
        let mut packet = Packet {
            header: Header::Authority {
                sequence: 9,
                binding: binding.clone(),
                authority: None,
            },
            payload: Vec::new(),
        };
        let mut bytes = Vec::new();
        write_packet(&mut bytes, &packet)?;
        assert!(
            matches!(read_packet(&mut bytes.as_slice())?.header,Header::Authority {sequence:9,binding:b,authority:None} if b==binding)
        );
        packet.payload.push(1);
        assert!(write_packet(&mut Vec::new(), &packet).is_err());
        Ok(())
    }
    #[test]
    fn refresh_unknown_injection_claim_is_rejected() {
        assert!(
            serde_json::from_str::<Command>(
                r#"{"command":"RefreshAuthority","sequence":9,"profile_digest":"a","input_seq":5}"#
            )
            .is_err()
        );
    }
}
