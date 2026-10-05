//! Exact bounded selected-source history. It supplies disposal identity only;
//! never a frame ticket, current decoder config, input grant or acknowledgment.
use super::*;
use std::collections::VecDeque;
const MAX_RETIRED: usize = 16;
#[derive(Default)]
pub(super) struct History(VecDeque<Record>);
struct Record {
    selected: Selected,
    config: Option<VideoConfig>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Disposition {
    Current,
    Retired(usize),
}
impl History {
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
    pub(super) fn remember(&mut self, selected: Selected, config: Option<VideoConfig>) {
        if self.0.len() == MAX_RETIRED {
            self.0.pop_front();
        }
        self.0.push_back(Record { selected, config });
    }
    pub(super) fn latest_generation(&self) -> u64 {
        self.0
            .iter()
            .map(|v| v.selected.scope.source_generation)
            .max()
            .unwrap_or(0)
    }
    pub(super) fn config(&mut self, index: usize, config: VideoConfig) -> SessionResult<()> {
        let record = self.0.get_mut(index).ok_or(SessionError::Invalid)?;
        if config.scope != record.selected.scope
            || config.visible_width != record.selected.target.client_rect.width
            || config.visible_height != record.selected.target.client_rect.height
            || record.config.as_ref().is_some_and(|old| old != &config)
        {
            return Err(SessionError::Invalid);
        }
        record.config = Some(config);
        Ok(())
    }
    pub(super) fn frame_context(
        &self,
        index: usize,
    ) -> SessionResult<(Selected, Option<VideoConfig>)> {
        let record = self.0.get(index).ok_or(SessionError::Invalid)?;
        Ok((record.selected.clone(), record.config.clone()))
    }
}
pub(super) fn disposition(s: &State, scope: &Scope) -> SessionResult<Disposition> {
    scope.validate().map_err(failure)?;
    if s.source_epoch != Some(scope.connection_epoch) {
        return Err(SessionError::Invalid);
    }
    if s.selected.as_ref().is_some_and(|v| &v.scope == scope) {
        return Ok(Disposition::Current);
    }
    s.retired
        .0
        .iter()
        .position(|v| &v.selected.scope == scope)
        .map(Disposition::Retired)
        .ok_or(SessionError::Invalid)
}
// Prospective MEDIA is disposal only. No source epoch is learned from it, and
// known source epochs plus current/retired generation floors remain fences.
// The caller retains one bool, never an untrusted frame or scope allocation.
pub(super) fn may_precede_start(s: &State, scope: &Scope) -> SessionResult<bool> {
    scope.validate().map_err(failure)?;
    if s.source_epoch
        .is_some_and(|epoch| epoch != scope.connection_epoch)
    {
        return Ok(false);
    }
    let floor = s
        .selected
        .as_ref()
        .map_or(0, |v| v.scope.source_generation)
        .max(s.retired.latest_generation());
    Ok(scope.source_generation > floor)
}
pub(super) fn control_shape(v: &pb::RemoteControl, host: bool) -> SessionResult<()> {
    let known = if host {
        matches!(
            v.action.as_str(),
            "request_control" | "keyframe" | "background" | "pause" | "stop" | "command_refused"
        )
    } else {
        matches!(
            v.action.as_str(),
            "grant" | "retired" | "command_request" | "pause" | "revoke" | "stop" | "authority"
        )
    };
    if !known
        || !v.selected_target_json.is_empty()
        || !v.selected_destination_label.is_empty()
        || (!matches!(v.action.as_str(), "grant" | "authority")
            && !v.shortcut_state_json.is_empty())
    {
        return Err(SessionError::Invalid);
    }
    if v.action == "grant" || v.action == "command_request" {
        let _ = Id::from_proto(v.input_session_id.as_ref()).map_err(|_| SessionError::Invalid)?;
    } else if let Some(id) = &v.input_session_id {
        let _ = Id::from_proto(Some(id)).map_err(|_| SessionError::Invalid)?;
    }
    if matches!(v.action.as_str(), "command_request" | "command_refused") {
        if v.request_nonce == 0 || !(1..=16).contains(&v.editor_action) {
            return Err(SessionError::Invalid);
        }
    } else if v.action == "authority" {
        if !(1..=16).contains(&v.editor_action) {
            return Err(SessionError::Invalid);
        }
    } else if v.request_nonce != 0 || v.editor_action != 0 {
        return Err(SessionError::Invalid);
    }
    if (v.action == "authority") != (v.command_input_seq > 0) {
        return Err(SessionError::Invalid);
    }
    if !v.shortcut_state_json.is_empty() {
        let _: vw_remote::profile::Authority =
            serde_json::from_slice(&v.shortcut_state_json).map_err(|_| SessionError::Invalid)?;
    }
    Ok(())
}
// A known retired target can validate structural disposal without inventing
// missing codec parameter sets. This function never returns a Frame/config.
pub(super) fn unconfigured_frame(v: pb::VideoFrame, selected: &Selected) -> SessionResult<()> {
    let scope = scope_from(v.remote_scope)?;
    let r = selected.target.client_rect;
    if scope != selected.scope
        || v.codec != "hevc"
        || v.config_generation != 1
        || v.frame_id == 0
        || v.capture_session_id != Some(native_id(&scope.capture_session_id)?.to_proto())
        || v.visible_width != r.width
        || v.visible_height != r.height
        || r.width.checked_add(1).map(|n| n & !1) != Some(v.coded_width)
        || r.height.checked_add(1).map(|n| n & !1) != Some(v.coded_height)
        || v.coded_width > vw_remote::MAX_SIDE
        || v.coded_height > vw_remote::MAX_SIDE
        || v.pts_ns <= 0
        || v.pts_ns % 100 != 0
        || u64::try_from(v.pts_ns / 100).ok() != Some(v.captured_qpc_100ns)
    {
        return Err(SessionError::Invalid);
    }
    let geometry = v.geometry.ok_or(SessionError::Invalid)?;
    if geometry.source_kind != "window"
        || geometry.window_handle != selected.target.window
        || !geometry.monitor_id.is_empty()
        || geometry.client_rect_host
            != Some(pb::RectI {
                x: r.x,
                y: r.y,
                w: r.width as i32,
                h: r.height as i32,
            })
        || geometry.geometry_revision != scope.geometry_revision
        || geometry.dpi_scale != f64::from(selected.target.dpi) / 96.0
        || geometry.timestamp_ns != v.pts_ns
    {
        return Err(SessionError::Invalid);
    }
    if let Some(id) = v.input_session_id {
        Id::from_proto(Some(&id)).map_err(|_| SessionError::Invalid)?;
    } else if v.last_input_seq_applied != 0 {
        return Err(SessionError::Invalid);
    }
    if vw_remote::annexb::access_unit(&v.annexb)
        .map_err(failure)?
        .idr
        != v.keyframe
    {
        return Err(SessionError::Invalid);
    }
    Ok(())
}
pub(super) fn frame(
    v: pb::VideoFrame,
    selected: &Selected,
    config: &VideoConfig,
) -> SessionResult<Frame> {
    let scope = scope_from(v.remote_scope)?;
    if v.codec != "hevc"
        || scope != selected.scope
        || config.scope != scope
        || v.capture_session_id != Some(native_id(&scope.capture_session_id)?.to_proto())
        || v.coded_width != config.coded_width
        || v.coded_height != config.coded_height
        || v.visible_width != config.visible_width
        || v.visible_height != config.visible_height
        || v.pts_ns <= 0
        || v.pts_ns % 100 != 0
        || u64::try_from(v.pts_ns / 100).ok() != Some(v.captured_qpc_100ns)
    {
        return Err(SessionError::Invalid);
    }
    let geometry = v.geometry.ok_or(SessionError::Invalid)?;
    let r = selected.target.client_rect;
    if geometry.source_kind != "window"
        || geometry.window_handle != selected.target.window
        || !geometry.monitor_id.is_empty()
        || geometry.client_rect_host
            != Some(pb::RectI {
                x: r.x,
                y: r.y,
                w: r.width as i32,
                h: r.height as i32,
            })
        || geometry.geometry_revision != scope.geometry_revision
        || geometry.dpi_scale != f64::from(selected.target.dpi) / 96.0
        || geometry.timestamp_ns != v.pts_ns
    {
        return Err(SessionError::Invalid);
    }
    let frame = Frame {
        scope,
        config_generation: v.config_generation,
        frame_id: v.frame_id,
        pts_100ns: v.pts_ns / 100,
        captured_qpc_100ns: v.captured_qpc_100ns,
        input_session_id: v
            .input_session_id
            .map(|id| {
                Id::from_proto(Some(&id))
                    .map(|v| v.to_string())
                    .map_err(|_| SessionError::Invalid)
            })
            .transpose()?,
        last_input_seq_applied: v.last_input_seq_applied,
        keyframe: v.keyframe,
        annexb: v.annexb,
    };
    frame.validate(config).map_err(failure)?;
    Ok(frame)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn id(n: u64) -> String {
        format!("01900000-0000-7000-8000-{n:012x}")
    }
    fn selected(n: u64) -> Selected {
        let rect = vw_remote::Rect {
            x: 10,
            y: 20,
            width: 800,
            height: 600,
        };
        Selected {
            target: Target {
                token: id(n * 2),
                window: n,
                process_id: 7,
                thread_id: 8,
                process_created: 9,
                window_rect: rect,
                frame_rect: rect,
                client_rect: rect,
                dpi: 96,
                integrity: 0,
            },
            scope: Scope {
                connection_epoch: 1,
                capture_session_id: id(n * 2 + 1),
                source_generation: n,
                target_token: id(n * 2),
                geometry_revision: 1,
            },
            display_label: "owned fixture".into(),
        }
    }
    // Same Main8 SPS fixture bit layout as vw-remote/tests/admission.rs.
    fn sps(width: u32, height: u32) -> Vec<u8> {
        fn bits(out: &mut Vec<bool>, value: u32, n: usize) {
            for bit in (0..n).rev() {
                out.push(value & (1 << bit) != 0)
            }
        }
        fn ue(out: &mut Vec<bool>, value: u32) {
            let v = value + 1;
            let n = (32 - v.leading_zeros()) as usize;
            out.extend(std::iter::repeat_n(false, n - 1));
            bits(out, v, n)
        }
        let mut b = Vec::new();
        bits(&mut b, 1, 8);
        bits(&mut b, 1, 8);
        bits(&mut b, 0, 32);
        bits(&mut b, 0, 24);
        bits(&mut b, 0, 24);
        bits(&mut b, 120, 8);
        ue(&mut b, 0);
        ue(&mut b, 1);
        ue(&mut b, width);
        ue(&mut b, height);
        bits(&mut b, 0, 1);
        ue(&mut b, 0);
        ue(&mut b, 0);
        b.push(true);
        let mut bytes = vec![0, 0, 0, 1, 66, 1];
        let mut zeros = 0;
        for chunk in b.chunks(8) {
            let mut byte = 0;
            for (n, bit) in chunk.iter().enumerate() {
                if *bit {
                    byte |= 1 << (7 - n)
                }
            }
            if zeros >= 2 && byte <= 3 {
                bytes.push(3);
                zeros = 0;
            }
            bytes.push(byte);
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        bytes
    }
    fn config(s: &Selected) -> VideoConfig {
        VideoConfig {
            scope: s.scope.clone(),
            generation: 1,
            visible_width: 800,
            visible_height: 600,
            coded_width: 800,
            coded_height: 600,
            vps: vec![0, 0, 0, 1, 64, 1, 1],
            sps: sps(800, 600),
            pps: vec![0, 0, 0, 1, 68, 1, 1],
            encoder: vw_remote::EncoderCapabilities {
                name: "fixture".into(),
                vendor_attribute: Some("VEN_8086".into()),
                adapter_vendor: 0x8086,
                hardware_enumerated: true,
                intel_vendor_confirmed: true,
                low_latency_control_accepted: true,
                zero_b_control_accepted: true,
                cbr_control_accepted: true,
                maximum_bitrate_control_accepted: true,
                bitrate: vw_remote::BITRATE,
                one_frame_in_flight: 1,
            },
        }
    }
    fn config_wire(c: &VideoConfig) -> SessionResult<pb::RemoteVideoConfig> {
        Ok(pb::RemoteVideoConfig {
            scope: Some(scope_pb(&c.scope)?),
            generation: c.generation,
            visible_width: c.visible_width,
            visible_height: c.visible_height,
            coded_width: c.coded_width,
            coded_height: c.coded_height,
            vps: c.vps.clone(),
            sps: c.sps.clone(),
            pps: c.pps.clone(),
            encoder_capabilities_json: serde_json::to_vec(&c.encoder)
                .map_err(|_| SessionError::Invalid)?,
        })
    }
    fn start(s: &Selected, sequence: u64) -> SessionResult<Body> {
        Ok(Body::RemoteControl(pb::RemoteControl {
            scope: Some(scope_pb(&s.scope)?),
            sequence,
            action: "start".into(),
            selected_target_json: serde_json::to_vec(&s.target)
                .map_err(|_| SessionError::Invalid)?,
            selected_destination_label: s.display_label.clone(),
            ..Default::default()
        }))
    }
    fn video(s: &Selected, id: u64) -> SessionResult<pb::VideoFrame> {
        let r = s.target.client_rect;
        Ok(pb::VideoFrame {
            frame_id: id,
            pts_ns: 10000,
            captured_qpc_100ns: 100,
            codec: "hevc".into(),
            annexb: vec![0, 0, 0, 1, 38, 1, 0x80],
            keyframe: true,
            remote_scope: Some(scope_pb(&s.scope)?),
            capture_session_id: Some(native_id(&s.scope.capture_session_id)?.to_proto()),
            coded_width: 800,
            coded_height: 600,
            visible_width: 800,
            visible_height: 600,
            config_generation: 1,
            geometry: Some(pb::CaptureGeometry {
                source_kind: "window".into(),
                window_handle: s.target.window,
                client_rect_host: Some(pb::RectI {
                    x: r.x,
                    y: r.y,
                    w: r.width as i32,
                    h: r.height as i32,
                }),
                dpi_scale: 1.0,
                geometry_revision: 1,
                timestamp_ns: 10000,
                ..Default::default()
            }),
            ..Default::default()
        })
    }
    fn hub(host: bool) -> SessionResult<Arc<RemoteHub>> {
        let hub = RemoteHub::new(host);
        {
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            s.epoch = 1;
            s.source_epoch = host.then_some(1);
            s.enabled = true;
        }
        Ok(hub)
    }
    fn receive(h: &Arc<RemoteHub>, body: Body) -> SessionResult<()> {
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        h.receive_locked(body, s)
    }
    fn a_then_b(h: &Arc<RemoteHub>) -> SessionResult<(Selected, Selected)> {
        let a = selected(1);
        let b = selected(2);
        receive(h, start(&a, 1)?)?;
        receive(h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        receive(h, Body::VideoFrame(video(&a, 1)?))?;
        receive(h, start(&b, 2)?)?;
        receive(h, Body::RemoteVideoConfig(config_wire(&config(&b))?))?;
        Ok((a, b))
    }
    #[test]
    fn independent_control_and_media_delayed_a_cannot_replace_b_or_disconnect_epoch()
    -> SessionResult<()> {
        let h = hub(false)?;
        let (a, b) = a_then_b(&h)?;
        receive(&h, Body::VideoFrame(video(&a, 2)?))?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        {
            let s = h.state.lock().map_err(|_| SessionError::Worker)?;
            assert_eq!(s.epoch, 1);
            assert!(s.enabled);
            assert_eq!(s.selected.as_ref().map(|v| &v.scope), Some(&b.scope));
            assert!(s.held.is_empty());
            assert!(s.ready.is_empty());
            assert_eq!(s.next_ticket, 1);
            assert_eq!(s.last_frame, 0);
            assert!(s.binding.is_none());
        }
        receive(&h, Body::VideoFrame(video(&b, 1)?))?;
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.last_frame, 1);
        assert_eq!(s.next_ticket, 2);
        assert_eq!(s.held.get(&2).map(|v| &v.scope), Some(&b.scope));
        assert_eq!(s.ready.front().copied(), Some(2));
        Ok(())
    }
    #[test]
    fn recognized_retired_frame_still_requires_original_geometry_clock_and_hevc_structure()
    -> SessionResult<()> {
        let h = hub(false)?;
        let (a, b) = a_then_b(&h)?;
        receive(&h, Body::VideoFrame(video(&b, 1)?))?;
        for mode in 0..4 {
            let mut v = video(&a, 2)?;
            match mode {
                0 => v.captured_qpc_100ns += 1,
                1 => v.annexb = vec![0, 0, 0, 1, 38, 0],
                2 => {
                    v.geometry
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .window_handle = b.target.window
                }
                _ => v.config_generation += 1,
            };
            assert!(receive(&h, Body::VideoFrame(v)).is_err());
        }
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.ready.front().copied(), Some(2));
        assert_eq!(s.next_ticket, 2);
        assert_eq!(s.held.len(), 1);
        Ok(())
    }
    #[test]
    fn lower_but_unknown_and_cross_epoch_scopes_remain_refused() -> SessionResult<()> {
        let h = hub(false)?;
        let (a, b) = a_then_b(&h)?;
        for mode in 0..2 {
            let mut foreign = a.clone();
            match mode {
                0 => foreign.scope.capture_session_id = id(99),
                _ => foreign.scope.connection_epoch = 3,
            };
            assert!(receive(&h, Body::VideoFrame(video(&foreign, 1)?)).is_err());
        }
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.epoch, 1);
        assert_eq!(s.selected.as_ref().map(|v| &v.scope), Some(&b.scope));
        assert_eq!(s.next_ticket, 1);
        Ok(())
    }
    #[test]
    fn retired_config_cannot_change_immutable_config_or_adopt_current_decoder_state()
    -> SessionResult<()> {
        let h = hub(false)?;
        let (a, b) = a_then_b(&h)?;
        let mut wrong = config(&a);
        wrong.generation = 2;
        assert!(receive(&h, Body::RemoteVideoConfig(config_wire(&wrong)?)).is_err());
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.config.as_ref().map(|c| &c.scope), Some(&b.scope));
        Ok(())
    }
    #[test]
    fn old_peer_control_requests_do_not_create_current_grants_or_commands() -> SessionResult<()> {
        let h = hub(false)?;
        let (a, b) = a_then_b(&h)?;
        for (sequence, action, nonce, editor_action) in [
            (3, "grant", 0, 0),
            (4, "command_request", 8, 1),
            (5, "revoke", 0, 0),
        ] {
            receive(
                &h,
                Body::RemoteControl(pb::RemoteControl {
                    scope: Some(scope_pb(&a.scope)?),
                    sequence,
                    action: action.into(),
                    input_session_id: Some(native_id(&id(50))?.to_proto()),
                    request_nonce: nonce,
                    editor_action,
                    ..Default::default()
                }),
            )?;
        }
        assert!(receive(&h, start(&a, 6)?).is_err());
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(s.binding.is_none());
        assert!(s.commands.is_empty());
        assert_eq!(s.selected.as_ref().map(|v| &v.scope), Some(&b.scope));
        Ok(())
    }
    #[test]
    fn bounded_history_never_manufactures_scope_from_generation_and_epoch_reset_clears_it()
    -> SessionResult<()> {
        let h = hub(false)?;
        let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
        for n in 1..=20 {
            RemoteHub::replace_selected(&mut s, selected(n))?;
            s.config = Some(config(&selected(n)));
        }
        assert_eq!(s.retired.0.len(), MAX_RETIRED);
        assert!(disposition(&s, &selected(1).scope).is_err());
        assert!(matches!(
            disposition(&s, &selected(19).scope)?,
            Disposition::Retired(_)
        ));
        s.retired.clear();
        assert!(disposition(&s, &selected(19).scope).is_err());
        Ok(())
    }
    #[cfg(windows)]
    #[test]
    fn old_keyframe_and_request_control_do_not_target_the_new_worker_or_grant() -> SessionResult<()>
    {
        use super::super::stream::Keyframes;
        let h = hub(true)?;
        let a = selected(1);
        let b = selected(2);
        {
            let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
            RemoteHub::replace_selected(&mut s, a.clone())?;
            RemoteHub::replace_selected(&mut s, b.clone())?;
        }
        let flag = Arc::new(Keyframes::default());
        h.workers
            .lock()
            .map_err(|_| SessionError::Worker)?
            .keyframe_fixture(flag.clone());
        for (sequence, action) in [(1, "keyframe"), (2, "request_control")] {
            receive(
                &h,
                Body::RemoteControl(pb::RemoteControl {
                    scope: Some(scope_pb(&a.scope)?),
                    sequence,
                    action: action.into(),
                    ..Default::default()
                }),
            )?;
        }
        assert_eq!(flag.generation(), 0);
        receive(
            &h,
            Body::RemoteControl(pb::RemoteControl {
                scope: Some(scope_pb(&b.scope)?),
                sequence: 3,
                action: "keyframe".into(),
                ..Default::default()
            }),
        )?;
        assert_eq!(flag.generation(), 1);
        Ok(())
    }
    #[test]
    fn endpoint_local_epoch_does_not_replace_host_source_epoch() -> SessionResult<()> {
        let h = hub(false)?;
        h.state.lock().map_err(|_| SessionError::Worker)?.epoch = 9;
        let (a, b) = a_then_b(&h)?;
        receive(&h, Body::VideoFrame(video(&a, 2)?))?;
        receive(&h, Body::VideoFrame(video(&b, 1)?))?;
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.epoch, 9);
        assert_eq!(s.source_epoch, Some(1));
        assert_eq!(s.ready.front().copied(), Some(2));
        assert_eq!(s.held.get(&2).map(|v| &v.scope), Some(&b.scope));
        Ok(())
    }
    #[test]
    fn source_epoch_change_requires_new_carrier_and_validated_start() -> SessionResult<()> {
        let h = hub(false)?;
        let (a, b) = a_then_b(&h)?;
        let mut foreign = selected(3);
        foreign.scope.connection_epoch = 9;
        assert!(receive(&h, start(&foreign, 3)?).is_err());
        {
            let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
            assert_eq!(s.source_epoch, Some(1));
            assert_eq!(s.selected.as_ref().map(|v| &v.scope), Some(&b.scope));
            assert!(matches!(
                disposition(&s, &a.scope)?,
                Disposition::Retired(_)
            ));
            // This is the exact source reset used by real activate(), without
            // creating a network sender or executing any native worker.
            RemoteHub::reset_source(&mut s, None);
            s.epoch = 3;
            s.config = None;
            s.received_control = 0;
            assert!(s.retired.0.is_empty());
        }
        receive(&h, start(&foreign, 1)?)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&foreign))?))?;
        receive(&h, Body::VideoFrame(video(&foreign, 1)?))?;
        assert!(receive(&h, Body::VideoFrame(video(&a, 3)?)).is_err());
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.epoch, 3);
        assert_eq!(s.source_epoch, Some(9));
        assert_eq!(s.selected.as_ref().map(|v| &v.scope), Some(&foreign.scope));
        Ok(())
    }
    #[test]
    fn invalid_first_start_cannot_anchor_epoch_or_replace_state() -> SessionResult<()> {
        let h = hub(false)?;
        let mut a = selected(1);
        a.scope.connection_epoch = 7;
        a.target.token = id(99);
        assert!(receive(&h, start(&a, 1)?).is_err());
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(s.source_epoch.is_none());
        assert!(s.selected.is_none());
        assert!(s.retired.0.is_empty());
        assert_eq!(s.grant_revision, 0);
        assert_eq!(s.next_ticket, 0);
        Ok(())
    }
    #[test]
    fn host_selection_still_requires_its_own_source_epoch() -> SessionResult<()> {
        let h = hub(true)?;
        let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
        let mut foreign = selected(1);
        foreign.scope.connection_epoch = 3;
        assert!(RemoteHub::replace_selected(&mut s, foreign).is_err());
        assert!(s.selected.is_none());
        assert_eq!(s.source_epoch, Some(1));
        RemoteHub::replace_selected(&mut s, selected(1))?;
        assert_eq!(
            s.selected.as_ref().map(|v| v.scope.connection_epoch),
            Some(1)
        );
        Ok(())
    }

    // Exact state transition shared with close_view after its real retirement
    // gates. No network sender or native worker is fabricated by these tests.
    fn close_selected_view(h: &RemoteHub) -> SessionResult<()> {
        let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
        RemoteHub::retire(&mut s);
        RemoteHub::retire_selected(&mut s);
        Ok(())
    }
    fn retired_control(s: &Selected, sequence: u64) -> SessionResult<Body> {
        Ok(Body::RemoteControl(pb::RemoteControl {
            scope: Some(scope_pb(&s.scope)?),
            sequence,
            action: "retired".into(),
            ..Default::default()
        }))
    }
    #[test]
    fn closed_view_keeps_late_config_media_and_retired_control_disposal_identity()
    -> SessionResult<()> {
        let h = hub(false)?;
        let a = selected(1);
        receive(&h, start(&a, 1)?)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        receive(&h, Body::VideoFrame(video(&a, 1)?))?;
        close_selected_view(&h)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        receive(&h, Body::VideoFrame(video(&a, 2)?))?;
        receive(&h, retired_control(&a, 2)?)?;
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(s.selected.is_none());
        assert!(s.config.is_none());
        assert!(s.binding.is_none());
        assert!(s.held.is_empty());
        assert!(s.ready.is_empty());
        assert_eq!(s.next_ticket, 1);
        assert_eq!(s.retired.latest_generation(), 1);
        Ok(())
    }
    #[test]
    fn closed_view_late_packets_cannot_poison_replacement_or_restore_old_grant() -> SessionResult<()>
    {
        let h = hub(false)?;
        let a = selected(1);
        let b = selected(2);
        receive(&h, start(&a, 1)?)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        close_selected_view(&h)?;
        receive(&h, start(&b, 2)?)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&b))?))?;
        receive(&h, Body::VideoFrame(video(&b, 1)?))?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        receive(&h, Body::VideoFrame(video(&a, 2)?))?;
        receive(&h, retired_control(&a, 3)?)?;
        receive(
            &h,
            Body::RemoteControl(pb::RemoteControl {
                scope: Some(scope_pb(&a.scope)?),
                sequence: 4,
                action: "grant".into(),
                input_session_id: Some(native_id(&id(50))?.to_proto()),
                ..Default::default()
            }),
        )?;
        assert!(
            receive(
                &h,
                Body::InputEvent(pb::InputEvent {
                    remote_scope: Some(scope_pb(&a.scope)?),
                    input_session_id: Some(native_id(&id(50))?.to_proto()),
                    ..Default::default()
                })
            )
            .is_err()
        );
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.selected.as_ref().map(|v| &v.scope), Some(&b.scope));
        assert_eq!(s.config.as_ref().map(|v| &v.scope), Some(&b.scope));
        assert!(s.binding.is_none());
        assert_eq!(s.next_ticket, 1);
        assert_eq!(s.held.len(), 1);
        assert_eq!(s.held.get(&1).map(|v| &v.scope), Some(&b.scope));
        Ok(())
    }
    #[test]
    fn closed_before_configuration_can_learn_exact_late_config_without_reopening_view()
    -> SessionResult<()> {
        let h = hub(false)?;
        let a = selected(1);
        receive(&h, start(&a, 1)?)?;
        close_selected_view(&h)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        receive(&h, Body::VideoFrame(video(&a, 1)?))?;
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(s.selected.is_none());
        assert!(s.config.is_none());
        assert_eq!(s.next_ticket, 0);
        assert!(s.held.is_empty());
        Ok(())
    }
    #[test]
    fn close_preserves_generation_floor_but_new_carrier_reset_clears_it() -> SessionResult<()> {
        let h = hub(false)?;
        let a = selected(2);
        receive(&h, start(&a, 1)?)?;
        close_selected_view(&h)?;
        for (sequence, stale) in [(2, selected(1)), (3, a.clone())] {
            assert!(receive(&h, start(&stale, sequence)?).is_err());
        }
        {
            let s = h.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(s.selected.is_none());
            assert!(s.binding.is_none());
            assert_eq!(s.retired.latest_generation(), 2);
        }
        receive(&h, start(&selected(3), 4)?)?;
        close_selected_view(&h)?;
        {
            let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
            RemoteHub::reset_source(&mut s, None);
            s.epoch = 3;
            s.received_control = 0;
        }
        let mut fresh = selected(1);
        fresh.scope.connection_epoch = 5;
        receive(&h, start(&fresh, 1)?)?;
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.selected.as_ref().map(|v| &v.scope), Some(&fresh.scope));
        assert_eq!(s.retired.latest_generation(), 0);
        Ok(())
    }
    #[test]
    fn bounded_closed_history_retains_latest_floor_and_rejects_unknown_scopes() -> SessionResult<()>
    {
        let h = hub(false)?;
        for n in 1..=20 {
            receive(&h, start(&selected(n), n)?)?;
            close_selected_view(&h)?;
        }
        let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.retired.0.len(), MAX_RETIRED);
        assert_eq!(s.retired.latest_generation(), 20);
        assert!(RemoteHub::replace_selected(&mut s, selected(19)).is_err());
        let mut unknown = selected(20);
        unknown.scope.capture_session_id = id(99);
        assert!(disposition(&s, &unknown.scope).is_err());
        assert!(disposition(&s, &selected(1).scope).is_err());
        assert!(s.selected.is_none());
        Ok(())
    }

    #[test]
    fn initial_media_before_start_is_disposal_until_control_admits_scope_and_config()
    -> SessionResult<()> {
        let h = hub(false)?;
        let a = selected(1);
        receive(&h, Body::VideoFrame(video(&a, 1)?))?;
        {
            let s = h.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(s.source_epoch.is_none() && s.selected.is_none() && s.config.is_none());
            assert!(s.binding.is_none() && s.held.is_empty() && s.ready.is_empty());
            assert!(s.prestart_media_lost && !s.start_recovery_pending);
            assert_eq!((s.next_ticket, s.last_frame, s.input_seq), (0, 0, 0));
        }
        receive(&h, start(&a, 1)?)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        {
            let s = h.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(s.start_recovery_pending && s.awaiting_idr && !s.prestart_media_lost);
            assert_eq!(s.source_epoch, Some(a.scope.connection_epoch));
            assert_eq!(s.next_ticket, 0);
        }
        receive(&h, Body::VideoFrame(video(&a, 2)?))?;
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.held.len(), 1);
        assert_eq!(s.last_frame, 2);
        assert!(!s.awaiting_idr && s.binding.is_none());
        Ok(())
    }
    #[test]
    fn replacement_media_before_start_preserves_current_decoder_ticket_and_authority()
    -> SessionResult<()> {
        let h = hub(false)?;
        let a = selected(1);
        let b = selected(2);
        receive(&h, start(&a, 1)?)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        receive(&h, Body::VideoFrame(video(&a, 1)?))?;
        let binding = Binding {
            scope: a.scope.clone(),
            input_session_id: id(101),
        };
        h.state.lock().map_err(|_| SessionError::Worker)?.binding = Some(binding.clone());
        receive(&h, Body::VideoFrame(video(&b, 1)?))?;
        {
            let s = h.state.lock().map_err(|_| SessionError::Worker)?;
            assert_eq!(s.selected.as_ref().map(|v| &v.scope), Some(&a.scope));
            assert_eq!(s.config.as_ref().map(|v| &v.scope), Some(&a.scope));
            assert_eq!(s.held.len(), 1);
            assert_eq!(s.next_ticket, 1);
            assert!(s.prestart_media_lost);
            assert_eq!(s.binding.as_ref(), Some(&binding));
        }
        receive(&h, start(&b, 2)?)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&b))?))?;
        receive(&h, Body::VideoFrame(video(&b, 2)?))?;
        receive(&h, Body::VideoFrame(video(&a, 2)?))?;
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.selected.as_ref().map(|v| &v.scope), Some(&b.scope));
        assert_eq!(s.held.len(), 1);
        assert_eq!(s.next_ticket, 2);
        assert_eq!(s.held.get(&2).map(|v| &v.scope), Some(&b.scope));
        assert!(s.binding.is_none());
        Ok(())
    }
    #[test]
    fn unknown_future_media_cannot_adopt_epoch_scope_or_skip_control_validation()
    -> SessionResult<()> {
        let h = hub(false)?;
        let mut future = selected(99);
        future.scope.connection_epoch = 17;
        for n in 1..=20 {
            receive(&h, Body::VideoFrame(video(&future, n)?))?;
        }
        {
            let s = h.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(s.source_epoch.is_none() && s.selected.is_none() && s.config.is_none());
            assert!(s.held.is_empty() && s.ready.is_empty() && s.binding.is_none());
            assert!(s.prestart_media_lost);
            assert_eq!(s.next_ticket, 0);
        }
        let a = selected(1);
        receive(&h, start(&a, 1)?)?;
        assert!(receive(&h, Body::VideoFrame(video(&future, 21)?)).is_err());
        let mut stale = a.clone();
        stale.scope.capture_session_id = id(100);
        assert!(receive(&h, Body::VideoFrame(video(&stale, 1)?)).is_err());
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert_eq!(s.source_epoch, Some(1));
        assert_eq!(s.next_ticket, 0);
        Ok(())
    }
    #[test]
    fn control_first_needs_no_extra_repair_and_actual_carrier_reset_forgets_loss()
    -> SessionResult<()> {
        let h = hub(false)?;
        let a = selected(1);
        receive(&h, start(&a, 1)?)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        receive(&h, Body::VideoFrame(video(&a, 1)?))?;
        {
            let s = h.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(!s.prestart_media_lost && !s.start_recovery_pending);
            assert_eq!(s.held.len(), 1);
        }
        receive(&h, Body::VideoFrame(video(&selected(2), 1)?))?;
        let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(s.prestart_media_lost);
        RemoteHub::reset_source(&mut s, None);
        assert!(!s.prestart_media_lost && !s.start_recovery_pending);
        Ok(())
    }

    #[test]
    fn start_recovery_enqueues_exact_admitted_scope_once_only_after_config_and_keeps_failed_request()
    -> SessionResult<()> {
        let h = hub(false)?;
        let a = selected(1);
        receive(&h, Body::VideoFrame(video(&a, 1)?))?;
        let mut emitted = Vec::new();
        {
            let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
            RemoteHub::flush_start_recovery(&mut s, |body| {
                emitted.push(body);
                Ok(())
            })?;
            assert!(emitted.is_empty());
        }
        receive(&h, start(&a, 1)?)?;
        {
            let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
            RemoteHub::flush_start_recovery(&mut s, |body| {
                emitted.push(body);
                Ok(())
            })?;
            assert!(emitted.is_empty() && s.start_recovery_pending);
            assert_eq!(s.sent_control, 0);
        }
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
        let failed = RemoteHub::flush_start_recovery(&mut s, |body| {
            let Body::RemoteControl(v) = body else {
                return Err(SessionError::Invalid);
            };
            assert_eq!(v.action, "keyframe");
            assert_eq!(scope_from(v.scope)?, a.scope);
            Err(SessionError::Backpressure)
        });
        assert!(matches!(failed, Err(SessionError::Backpressure)));
        assert!(s.start_recovery_pending && s.awaiting_idr && s.held.is_empty());
        RemoteHub::flush_start_recovery(&mut s, |body| {
            emitted.push(body);
            Ok(())
        })?;
        assert!(!s.start_recovery_pending);
        RemoteHub::flush_start_recovery(&mut s, |body| {
            emitted.push(body);
            Ok(())
        })?;
        assert_eq!(emitted.len(), 1);
        let Some(Body::RemoteControl(v)) = emitted.pop() else {
            return Err(SessionError::Invalid);
        };
        assert_eq!(scope_from(v.scope)?, a.scope);
        assert_eq!(v.action, "keyframe");
        assert_eq!(v.sequence, 2);
        assert!(v.input_session_id.is_none());
        assert!(v.selected_target_json.is_empty() && v.shortcut_state_json.is_empty());
        assert_eq!(
            (v.command_input_seq, v.request_nonce, v.editor_action),
            (0, 0, 0)
        );
        assert!(s.awaiting_idr && s.held.is_empty() && s.binding.is_none());
        Ok(())
    }

    #[test]
    fn closed_before_config_late_media_then_config_never_adopts_and_replacement_renders()
    -> SessionResult<()> {
        let h = hub(false)?;
        let a = selected(1);
        let b = selected(2);
        receive(&h, start(&a, 1)?)?;
        close_selected_view(&h)?;
        receive(&h, Body::VideoFrame(video(&a, 1)?))?;
        {
            let s = h.state.lock().map_err(|_| SessionError::Worker)?;
            assert!(s.selected.is_none() && s.config.is_none() && s.binding.is_none());
            assert!(s.held.is_empty() && s.ready.is_empty());
            assert_eq!(s.next_ticket, 0);
            assert!(!s.prestart_media_lost && !s.start_recovery_pending);
        }
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&a))?))?;
        receive(&h, start(&b, 2)?)?;
        receive(&h, Body::RemoteVideoConfig(config_wire(&config(&b))?))?;
        receive(&h, Body::VideoFrame(video(&b, 1)?))?;
        receive(&h, Body::VideoFrame(video(&a, 2)?))?;
        let mut s = h.state.lock().map_err(|_| SessionError::Worker)?;
        let ticket = take_ready(&mut s).ok_or(SessionError::Invalid)?;
        let pts = s.held.get(&ticket).ok_or(SessionError::Invalid)?.pts_100ns / 10;
        assert!(acknowledge_issued(&mut s, ticket, pts, &b.scope, None, 0)?.is_none());
        assert!(s.held.is_empty() && s.binding.is_none());
        assert_eq!(s.config.as_ref().map(|v| &v.scope), Some(&b.scope));
        Ok(())
    }
    #[test]
    fn retired_without_config_rejects_malformed_structure_unknown_scope_and_cross_epoch()
    -> SessionResult<()> {
        let h = hub(false)?;
        let a = selected(1);
        receive(&h, start(&a, 1)?)?;
        close_selected_view(&h)?;
        for mode in 0..10 {
            let mut v = video(&a, 1)?;
            match mode {
                0 => v.pts_ns += 1,
                1 => v.annexb = vec![0, 0, 0, 1, 38, 0],
                2 => {
                    v.geometry
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .window_handle += 1
                }
                3 => v.config_generation = 2,
                4 => v.keyframe = false,
                5 => v.visible_width += 1,
                6 => v.coded_width += 2,
                7 => v.last_input_seq_applied = 1,
                8 => {
                    v.remote_scope
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .capture_session_id = Some(native_id(&id(105))?.to_proto())
                }
                _ => {
                    v.remote_scope
                        .as_mut()
                        .ok_or(SessionError::Invalid)?
                        .connection_epoch += 2
                }
            }
            assert!(receive(&h, Body::VideoFrame(v)).is_err());
        }
        let s = h.state.lock().map_err(|_| SessionError::Worker)?;
        assert!(s.selected.is_none() && s.config.is_none() && s.binding.is_none());
        assert!(s.held.is_empty() && s.ready.is_empty());
        assert_eq!(s.next_ticket, 0);
        Ok(())
    }
}
