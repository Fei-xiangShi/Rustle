//! Small, bounded XML tree for lyric containers. Entity references are events
//! in quick-xml 0.42 and must be decoded separately from text and CDATA.

use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use std::io::BufRead;

pub(super) enum Content {
    Text(String),
    Element(Node),
}

pub(super) struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Content>,
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

impl Node {
    fn new(event: &BytesStart<'_>) -> Result<Self, String> {
        let attrs = event
            .attributes()
            .map(|attr| {
                let attr = attr.map_err(|error| error.to_string())?;
                Ok((
                    local_name(attr.key.as_ref()).to_owned(),
                    // Preserve literal newlines in QQ's LyricContent attribute.
                    quick_xml::escape::unescape(&attr.value)
                        .map_err(|error| error.to_string())?
                        .into_owned(),
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            name: local_name(event.name().as_ref()).to_owned(),
            attrs,
            children: Vec::new(),
        })
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn elements(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().filter_map(|content| match content {
            Content::Element(node) => Some(node),
            Content::Text(_) => None,
        })
    }

    pub fn text(&self) -> String {
        let mut result = String::new();
        for child in &self.children {
            match child {
                Content::Text(text) => result.push_str(text),
                Content::Element(node) => result.push_str(&node.text()),
            }
        }
        result
    }

    pub fn visit(&self, visitor: &mut impl FnMut(&Node)) {
        visitor(self);
        for node in self.elements() {
            node.visit(visitor);
        }
    }
}

fn push_text(stack: &mut [Node], text: &str) -> Result<(), String> {
    let Some(node) = stack.last_mut() else {
        return if text.trim().is_empty() {
            Ok(())
        } else {
            Err("Text outside XML root".into())
        };
    };
    if let Some(Content::Text(previous)) = node.children.last_mut() {
        previous.push_str(text);
    } else {
        node.children.push(Content::Text(text.to_owned()));
    }
    Ok(())
}

fn push_node(stack: &mut [Node], root: &mut Option<Node>, node: Node) -> Result<(), String> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(Content::Element(node));
    } else if root.is_none() {
        *root = Some(node);
    } else {
        return Err("Multiple XML roots".into());
    }
    Ok(())
}

pub(super) fn parse(data: impl BufRead) -> Result<Node, String> {
    let mut reader = Reader::from_reader(data);
    let mut buffer = Vec::new();
    let mut stack = Vec::new();
    let mut root = None;
    let mut nodes = 0usize;
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| error.to_string())?;
        if matches!(event, Event::Start(_) | Event::Empty(_)) {
            nodes += 1;
            if nodes > 100_000 || stack.len() >= 128 {
                return Err("XML lyric nesting/size limit".into());
            }
        }
        match event {
            Event::Start(event) => stack.push(Node::new(&event)?),
            Event::Empty(event) => push_node(&mut stack, &mut root, Node::new(&event)?)?,
            Event::End(_) => {
                let node = stack.pop().ok_or("Unexpected XML closing tag")?;
                push_node(&mut stack, &mut root, node)?;
            }
            Event::Text(event) => push_text(&mut stack, event.as_ref())?,
            Event::CData(event) => push_text(&mut stack, event.as_ref())?,
            Event::GeneralRef(event) => {
                let encoded = format!("&{};", event.as_ref());
                let text =
                    quick_xml::escape::unescape(&encoded).map_err(|error| error.to_string())?;
                push_text(&mut stack, &text)?;
            }
            Event::DocType(_) => return Err("DTD is not supported in lyric XML".into()),
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !stack.is_empty() {
        return Err("Unclosed XML element".into());
    }
    root.ok_or_else(|| "Missing XML root".into())
}
