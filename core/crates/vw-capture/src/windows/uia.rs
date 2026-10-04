use super::{DpiScope, api, clock_ns, fixed_target};
use crate::*;
use ::windows::{
    Win32::{Foundation::*, System::Com::*, UI::Accessibility::*},
    core::{BSTR, Interface},
};
use std::time::{Duration, Instant};
struct Com;
impl Com {
    fn new() -> Result<Self> {
        /* SAFETY: this helper-owned thread initializes an MTA once. */
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|_| Error::Platform)?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        /* SAFETY: paired successful MTA initialization on this same thread. */
        unsafe {
            CoUninitialize();
        }
    }
}
fn text(value: BSTR, max: usize) -> Result<String> {
    if value.len() > max {
        return Err(Error::Limit);
    }
    let text = String::from_utf16(&value).map_err(|_| Error::Invalid)?;
    if text.len() > max || text.contains('\0') {
        Err(Error::Limit)
    } else {
        Ok(text)
    }
}
fn excerpt(value: BSTR) -> Result<String> {
    if value.len() > 400 {
        return Err(Error::Limit);
    }
    let units: &[u16] = &value;
    let end = units.len()
        - usize::from(
            units
                .last()
                .is_some_and(|last| (0xd800..=0xdbff).contains(last)),
        );
    let value = String::from_utf16(&units[..end]).map_err(|_| Error::Invalid)?;
    if value.contains('\0') {
        return Err(Error::Invalid);
    }
    Ok(value.chars().take(200).collect())
}
fn maybe(
    result: ::windows::core::Result<IUIAutomationElement>,
) -> Result<Option<IUIAutomationElement>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.code() == S_OK || error.code() == E_POINTER => Ok(None),
        Err(_) => Err(Error::Platform),
    }
}
fn check(start: Instant, limits: Limits, cancel: &Cancellation) -> Result<()> {
    cancel.check()?;
    if start.elapsed() > Duration::from_millis(limits.tree_ms) {
        Err(Error::Timeout)
    } else {
        Ok(())
    }
}
pub fn collect(request: TreeRequest, cancel: &Cancellation) -> Result<TreeReceipt> {
    request.limits.validate()?;
    request.frame.identity.validate()?;
    request.frame.target.validate(request.owner_process_id)?;
    vw_model::AssetId::try_from(request.frame.source_asset_id.clone())
        .map_err(|_| Error::Invalid)?;
    if request.frame.identity.platform != "windows"
        || request.frame.identity.window_handle != request.frame.target.window
        || request.frame.identity.client_rect != request.frame.target.client
        || request.frame.width != request.frame.target.client.width
        || request.frame.height != request.frame.target.client.height
        || !request.frame.lossless
    {
        return Err(Error::Invalid);
    }
    let _dpi = DpiScope::enter()?;
    request.frame.target.unchanged(
        &fixed_target(request.frame.target.window, request.owner_process_id)?,
        request.owner_process_id,
    )?;
    let _com = Com::new()?;
    let started = Instant::now();
    let collected_at = clock_ns()?;
    // SAFETY: COM wrappers own every interface. Caching is scoped to ONE element
    // per call, preventing FindAll/Subtree from allocating an unbounded tree.
    let (automation, cache, walker, root) = unsafe {
        let automation: IUIAutomation2 = api(CoCreateInstance(
            &CUIAutomation8,
            None,
            CLSCTX_INPROC_SERVER,
        ))?;
        api(automation.SetConnectionTimeout(request.limits.tree_ms as u32))?;
        api(automation.SetTransactionTimeout(request.limits.tree_ms as u32))?;
        let cache = api(automation.CreateCacheRequest())?;
        api(cache.SetTreeScope(TreeScope_Element))?;
        api(cache.SetAutomationElementMode(AutomationElementMode_Full))?;
        for property in [
            UIA_NamePropertyId,
            UIA_ControlTypePropertyId,
            UIA_AutomationIdPropertyId,
            UIA_BoundingRectanglePropertyId,
            UIA_IsEnabledPropertyId,
            UIA_HasKeyboardFocusPropertyId,
            UIA_IsPasswordPropertyId,
        ] {
            api(cache.AddProperty(property))?;
        }
        api(cache.AddPattern(UIA_TextPatternId))?;
        let walker = api(automation.RawViewWalker())?;
        let root = api(automation.ElementFromHandleBuildCache(
            HWND(request.frame.target.window as usize as *mut _),
            &cache,
        ))?;
        (automation, cache, walker, root)
    };
    let _automation = automation;
    let mut stack = vec![(root, None::<String>, 0usize)];
    let mut elements = Vec::new();
    let mut retained = 0;
    while let Some((node, parent, depth)) = stack.pop() {
        check(started, request.limits, cancel)?;
        if elements.len() >= request.limits.max_elements || depth >= 128 {
            return Err(Error::Limit);
        }
        let local_id = format!("node-{}", elements.len());
        // SAFETY: only cached properties of this live element are read. Password
        // text is never requested; a bounded400 UTF-16-unit range is trimmed to
        // at most200 Unicode scalars without splitting a surrogate pair.
        let element = unsafe {
            let name = text(api(node.CachedName())?, 4096)?;
            let automation_id = text(api(node.CachedAutomationId())?, 1024)?;
            let kind = api(node.CachedControlType())?.0;
            let rect = api(node.CachedBoundingRectangle())?;
            let width = i64::from(rect.right) - i64::from(rect.left);
            let height = i64::from(rect.bottom) - i64::from(rect.top);
            if width < 0 || height < 0 {
                return Err(Error::Invalid);
            }
            let password = api(node.CachedIsPassword())?.as_bool();
            let captured_text = if password {
                String::new()
            } else {
                match node.GetCachedPattern(UIA_TextPatternId) {
                    Ok(value) => {
                        let pattern: IUIAutomationTextPattern = api(value.cast())?;
                        let range = api(pattern.DocumentRange())?;
                        excerpt(api(range.GetText(400))?)?
                    }
                    Err(error)
                        if error.code().0 as u32 == UIA_E_NOTSUPPORTED
                            || error.code() == E_NOINTERFACE
                            || error.code() == E_POINTER
                            || error.code() == S_OK =>
                    {
                        String::new()
                    }
                    Err(_) => return Err(Error::Platform),
                }
            };
            Element {
                local_id: local_id.clone(),
                parent_local_id: parent.clone(),
                name,
                role: role(kind),
                automation_id: (!automation_id.is_empty()).then_some(automation_id),
                resource_id: None,
                html_id: None,
                bounds: [
                    f64::from(rect.left),
                    f64::from(rect.top),
                    width as f64,
                    height as f64,
                ],
                text: captured_text,
                enabled: api(node.CachedIsEnabled())?.as_bool(),
                focused: api(node.CachedHasKeyboardFocus())?.as_bool(),
            }
        };
        admit_element(&element, &mut retained, request.limits)?;
        elements.push(element);
        check(started, request.limits, cancel)?;
        // SAFETY: tree walker calls stay inside the captured root. Siblings of
        // the root itself are deliberately never requested.
        unsafe {
            if parent.is_some()
                && let Some(sibling) = maybe(walker.GetNextSiblingElementBuildCache(&node, &cache))?
            {
                stack.push((sibling, parent, depth));
            }
            if let Some(child) = maybe(walker.GetFirstChildElementBuildCache(&node, &cache))? {
                stack.push((child, Some(local_id), depth + 1));
            }
        }
    }
    check(started, request.limits, cancel)?;
    request.frame.target.unchanged(
        &fixed_target(request.frame.target.window, request.owner_process_id)?,
        request.owner_process_id,
    )?;
    let elapsed = u64::try_from(started.elapsed().as_millis()).map_err(|_| Error::Limit)?;
    if elapsed > request.limits.tree_ms {
        return Err(Error::Timeout);
    }
    Ok(TreeReceipt {
        frame_delta_ms: frame_delta_ms(
            request.frame.identity.monotonic_timestamp_ns,
            collected_at,
        )?,
        identity: request.frame.identity,
        source_asset_id: request.frame.source_asset_id,
        platform: "uia".into(),
        collection_elapsed_ms: elapsed,
        elements,
    })
}
fn role(kind: i32) -> String {
    [
        (UIA_ButtonControlTypeId, "button"),
        (UIA_CheckBoxControlTypeId, "checkbox"),
        (UIA_ComboBoxControlTypeId, "combobox"),
        (UIA_EditControlTypeId, "edit"),
        (UIA_HyperlinkControlTypeId, "hyperlink"),
        (UIA_ImageControlTypeId, "image"),
        (UIA_ListItemControlTypeId, "list_item"),
        (UIA_ListControlTypeId, "list"),
        (UIA_MenuItemControlTypeId, "menu_item"),
        (UIA_RadioButtonControlTypeId, "radio_button"),
        (UIA_SliderControlTypeId, "slider"),
        (UIA_TabControlTypeId, "tab"),
        (UIA_TabItemControlTypeId, "tab_item"),
        (UIA_TextControlTypeId, "text"),
        (UIA_DocumentControlTypeId, "document"),
        (UIA_SplitButtonControlTypeId, "split_button"),
        (UIA_WindowControlTypeId, "window"),
        (UIA_PaneControlTypeId, "pane"),
        (UIA_GroupControlTypeId, "group"),
    ]
    .into_iter()
    .find_map(|(id, label)| (id.0 == kind).then_some(label.to_owned()))
    .unwrap_or_else(|| format!("control_type_{kind}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_control_ids_are_not_interchanged() {
        assert_eq!(role(UIA_DocumentControlTypeId.0), "document");
        assert_eq!(role(UIA_PaneControlTypeId.0), "pane");
        assert_eq!(role(UIA_GroupControlTypeId.0), "group");
    }
    #[test]
    fn emoji_excerpt_is_scalar_bounded() {
        let value = "😀".repeat(200);
        assert_eq!(excerpt(BSTR::from(value.as_str())).unwrap(), value);
        assert_eq!(
            excerpt(BSTR::from("a".repeat(400).as_str())).unwrap().len(),
            200
        );
    }
}
