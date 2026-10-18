//! XML is parsed with DTD/external-entity resolution disabled before usvg sees it.
use std::collections::{BTreeMap, BTreeSet};
use usvg::roxmltree::{Document, Node, ParsingOptions};

use crate::{Limits, SvgError, SvgResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FidelityNotice {
    ScriptOrEventIgnored,
    ForeignObjectIgnored,
    AnimationIgnored,
    ResourceDenied,
    StylesheetLimited,
    UnsupportedFilter,
    UnsupportedElement,
    MissingFont,
    MissingGlyph,
}

pub(crate) struct Admission<'a> {
    pub doc: Document<'a>,
    pub notices: BTreeSet<FidelityNotice>,
    pub workspace: u64,
    pub layers: u64,
    pub filters: u64,
}

pub(crate) fn inspect<'a>(
    text: &'a str,
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> SvgResult<Admission<'a>> {
    limits.validate()?;
    if cancelled() {
        return Err(SvgError::Cancelled);
    }
    if text.len() > limits.input_bytes {
        return Err(SvgError::Limit);
    }
    // roxmltree reserves from these lexical counts before applying nodes_limit.
    // Bound those reservations before invoking it, including misleading comments.
    let opens = text.bytes().filter(|b| *b == b'<').count();
    let attributes = text.bytes().filter(|b| *b == b'=').count();
    if opens > limits.nodes as usize * 2 || attributes > limits.nodes as usize * 32 {
        return Err(SvgError::Limit);
    }
    limits.parse_memory(text.len(), text.len() as u64)?;
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err(SvgError::DtdDenied);
    }
    let doc = Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: limits.nodes,
            entity_resolver: None,
        },
    )
    .map_err(|_| SvgError::Malformed)?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg"
        || !matches!(
            root.tag_name().namespace(),
            None | Some("http://www.w3.org/2000/svg")
        )
    {
        return Err(SvgError::Malformed);
    }
    let mut notices = BTreeSet::new();
    let mut ids = BTreeMap::new();
    let mut layers = 0_u64;
    let mut filters = 0_u64;
    for node in doc.descendants() {
        if cancelled() {
            return Err(SvgError::Cancelled);
        }
        if node.ancestors().take(limits.depth as usize + 2).count() > limits.depth as usize + 1 {
            return Err(SvgError::Limit);
        }
        if node.is_pi() {
            notices.insert(FidelityNotice::StylesheetLimited);
        }
        if !node.is_element() {
            continue;
        }
        if let Some(id) = node.attribute("id") {
            if id.len() > 1024 || ids.insert(id, node).is_some() {
                return Err(SvgError::Malformed);
            }
        }
        layers = layers.checked_add(1).ok_or(SvgError::Limit)?;
        let name = node.tag_name().name();
        match name {
            "script" => {
                notices.insert(FidelityNotice::ScriptOrEventIgnored);
            }
            "foreignObject" => {
                notices.insert(FidelityNotice::ForeignObjectIgnored);
            }
            "animate" | "animateMotion" | "animateTransform" | "set" | "discard" => {
                notices.insert(FidelityNotice::AnimationIgnored);
            }
            "image" | "feImage" => {
                notices.insert(FidelityNotice::ResourceDenied);
            }
            "style" => {
                notices.insert(FidelityNotice::StylesheetLimited);
            }
            "svg" | "g" | "defs" | "path" | "rect" | "circle" | "ellipse" | "line" | "polyline"
            | "polygon" | "text" | "tspan" | "textPath" | "a" | "title" | "desc" | "metadata"
            | "use" | "symbol" | "switch" | "linearGradient" | "radialGradient" | "stop"
            | "clipPath" | "mask" | "pattern" | "marker" | "filter" => {}
            value if value.starts_with("fe") => {
                filters = filters.checked_add(1).ok_or(SvgError::Limit)?;
                if !matches!(
                    value,
                    "feBlend"
                        | "feColorMatrix"
                        | "feComponentTransfer"
                        | "feComposite"
                        | "feConvolveMatrix"
                        | "feDiffuseLighting"
                        | "feDisplacementMap"
                        | "feDistantLight"
                        | "feDropShadow"
                        | "feFlood"
                        | "feFuncA"
                        | "feFuncB"
                        | "feFuncG"
                        | "feFuncR"
                        | "feGaussianBlur"
                        | "feMerge"
                        | "feMergeNode"
                        | "feMorphology"
                        | "feOffset"
                        | "fePointLight"
                        | "feSpecularLighting"
                        | "feSpotLight"
                        | "feTile"
                        | "feTurbulence"
                ) {
                    notices.insert(FidelityNotice::UnsupportedFilter);
                }
            }
            _ => {
                notices.insert(FidelityNotice::UnsupportedElement);
            }
        }
        for attribute in node.attributes() {
            if attribute.name().starts_with("on") {
                notices.insert(FidelityNotice::ScriptOrEventIgnored);
            }
            if attribute.name() == "href" && !attribute.value().trim().starts_with('#') {
                notices.insert(FidelityNotice::ResourceDenied);
            }
            if attribute.name() == "style" {
                notices.insert(FidelityNotice::StylesheetLimited);
            }
            // No URL resolver is installed for paint resources either. Report
            // their loss explicitly, including references in presentation CSS.
            let mut rest = attribute.value();
            while let Some(at) = rest.find("url(") {
                rest = &rest[at + 4..];
                let Some(end) = rest.find(')') else {
                    return Err(SvgError::Malformed);
                };
                let target = rest[..end].trim().trim_matches(['\'', '"']);
                if !target.starts_with('#') {
                    notices.insert(FidelityNotice::ResourceDenied);
                }
                rest = &rest[end + 1..];
            }
        }
    }
    // The expanded byte budget charges each reference occurrence, not just each
    // distinct definition. This bounds acyclic exponential <use>/paint graphs.
    let mut work = Work {
        remaining: limits.expanded_bytes,
        visits: 0,
        limits,
        ids: &ids,
        cancelled,
    };
    work.visit(root, &mut Vec::new())?;
    let expanded = limits.expanded_bytes - work.remaining;
    let workspace = limits.parse_memory(text.len(), expanded.max(text.len() as u64))?;
    Ok(Admission {
        doc,
        notices,
        workspace,
        layers,
        filters,
    })
}

struct Work<'a, 'input, 'doc> {
    remaining: u64,
    visits: u64,
    limits: Limits,
    ids: &'a BTreeMap<&'input str, Node<'doc, 'input>>,
    cancelled: &'a dyn Fn() -> bool,
}

impl<'input, 'doc> Work<'_, 'input, 'doc> {
    fn charge(&mut self, bytes: usize) -> SvgResult<()> {
        self.remaining = self
            .remaining
            .checked_sub(bytes as u64)
            .ok_or(SvgError::Limit)?;
        Ok(())
    }
    fn visit(
        &mut self,
        node: Node<'doc, 'input>,
        stack: &mut Vec<usvg::roxmltree::NodeId>,
    ) -> SvgResult<()> {
        if (self.cancelled)() {
            return Err(SvgError::Cancelled);
        }
        self.visits = self.visits.checked_add(1).ok_or(SvgError::Limit)?;
        if self.visits > u64::from(self.limits.nodes) * 4
            || stack.len() >= self.limits.depth as usize
        {
            return Err(SvgError::Limit);
        }
        if stack.contains(&node.id()) {
            return Err(SvgError::ReferenceCycle);
        }
        stack.push(node.id());
        self.charge(128)?;
        if node.is_text() {
            self.charge(node.text().map_or(0, str::len))?;
        }
        for attr in node.attributes() {
            self.charge(attr.value().len())?;
            if attr.name() == "href" {
                if let Some(id) = attr.value().trim().strip_prefix('#') {
                    self.reference(id, stack)?;
                }
            }
            if matches!(
                attr.name(),
                "style"
                    | "fill"
                    | "stroke"
                    | "filter"
                    | "clip-path"
                    | "mask"
                    | "marker"
                    | "marker-start"
                    | "marker-mid"
                    | "marker-end"
            ) && (attr.value().contains('\\')
                || attr.value().contains('@')
                || attr.value().contains("/*"))
            {
                return Err(SvgError::UnsupportedCss);
            }
            if attr
                .value()
                .as_bytes()
                .windows(4)
                .any(|word| word.eq_ignore_ascii_case(b"url(") && word != b"url(")
            {
                return Err(SvgError::UnsupportedCss);
            }
            let mut rest = attr.value();
            while let Some(at) = rest.find("url(") {
                rest = &rest[at + 4..];
                let Some(end) = rest.find(')') else {
                    return Err(SvgError::Malformed);
                };
                let id = rest[..end].trim().trim_matches(['\'', '"']);
                if let Some(id) = id.strip_prefix('#') {
                    self.reference(id, stack)?;
                }
                rest = &rest[end + 1..];
            }
        }
        // Stylesheets can hide reference expansion behind selectors and escaped
        // CSS. Refuse reference-bearing sheets rather than guessing their graph.
        if node.is_element() && node.tag_name().name() == "style" {
            // XML comments/CDATA may split a CSS token across text nodes.
            // Refuse that subset before a downstream stylesheet concatenation.
            let mut texts = node.children();
            let first = texts.next();
            if texts.next().is_some() || first.is_some_and(|child| !child.is_text()) {
                return Err(SvgError::UnsupportedCss);
            }
            let css = first.and_then(|child| child.text()).unwrap_or_default();
            if css.contains('\\')
                || css.to_ascii_lowercase().contains("url(")
                || css.contains('@')
                || css.contains("/*")
            {
                return Err(SvgError::UnsupportedCss);
            }
        }
        for child in node.children() {
            self.visit(child, stack)?;
        }
        let _ = stack.pop();
        Ok(())
    }
    fn reference(&mut self, id: &str, stack: &mut Vec<usvg::roxmltree::NodeId>) -> SvgResult<()> {
        if let Some(target) = self.ids.get(id).copied() {
            self.visit(target, stack)?;
        }
        Ok(())
    }
}
