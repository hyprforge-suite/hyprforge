//! Serving `com.canonical.dbusmenu` — the right-click menu.
//!
//! The protocol is older and stranger than StatusNotifierItem, and one
//! detail carries most of the risk: `GetLayout` returns a **recursive**
//! structure, `(ia{sv}av)`, whose children are variants each wrapping
//! another one of the same. Getting that signature wrong does not fail —
//! the host reads a menu with no rows in it and shows an empty popup,
//! with nothing logged at either end.

use crate::menu::{ItemKind, Menu, MenuItem};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, StructureBuilder, Type, Value};

/// One node of `GetLayout`'s tree: `(ia{sv}av)`.
///
/// A concrete type rather than a `Structure`, because `Structure`'s
/// signature is only known at runtime and `zbus::interface` needs a
/// static one to advertise. Children are variants each wrapping another
/// `LayoutNode`, which is what makes the type recursive without being
/// recursive in Rust.
#[derive(Debug, Clone, serde::Serialize, Type)]
pub struct LayoutNode(i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

pub const MENU_PATH_SUFFIX: &str = "/Menu";

/// The properties a host asks for, per row.
fn properties(item: &MenuItem) -> HashMap<String, OwnedValue> {
    let mut props: HashMap<String, OwnedValue> = HashMap::new();

    // "standard" is the default and may be omitted; "separator" may not.
    if item.kind == ItemKind::Separator {
        props.insert(
            "type".to_string(),
            OwnedValue::try_from(Value::from("separator")).expect("a string is a value"),
        );
        return props;
    }

    props.insert(
        "label".to_string(),
        OwnedValue::try_from(Value::from(item.label.as_str())).expect("a string is a value"),
    );
    props.insert(
        "enabled".to_string(),
        OwnedValue::try_from(Value::from(item.enabled)).expect("a bool is a value"),
    );
    props.insert(
        "visible".to_string(),
        OwnedValue::try_from(Value::from(item.visible)).expect("a bool is a value"),
    );

    if let Some(on) = item.toggle {
        props.insert(
            "toggle-type".to_string(),
            OwnedValue::try_from(Value::from("checkmark")).expect("a string is a value"),
        );
        // 1 on, 0 off. The spec also has -1 for "indeterminate", which
        // nothing here produces.
        props.insert(
            "toggle-state".to_string(),
            OwnedValue::try_from(Value::from(if on { 1i32 } else { 0i32 })).expect("an int is a value"),
        );
    }

    if !item.children.is_empty() {
        // Without this a host does not know the row opens a submenu and
        // draws it as an ordinary entry that does nothing.
        props.insert(
            "children-display".to_string(),
            OwnedValue::try_from(Value::from("submenu")).expect("a string is a value"),
        );
    }

    props
}

/// One node of the layout tree.
///
/// `depth` counts down; `-1` means "everything", which is what every host
/// actually asks for.
fn node(
    id: i32,
    props: HashMap<String, OwnedValue>,
    children: &[MenuItem],
    depth: i32,
) -> LayoutNode {
    let kids: Vec<OwnedValue> = if depth == 0 {
        Vec::new()
    } else {
        children
            .iter()
            .map(|child| {
                let child_node = node(
                    child.id,
                    properties(child),
                    &child.children,
                    depth.saturating_sub(1),
                );
                into_value(child_node)
            })
            .collect()
    };

    LayoutNode(id, props, kids)
}

/// Wraps a node in the variant its parent's `av` needs.
///
/// Built by hand because a derived `Serialize` gives no way *into* a
/// `Value`, and the child level of this protocol is variants all the way
/// down.
fn into_value(node: LayoutNode) -> OwnedValue {
    let structure = StructureBuilder::new()
        .add_field(node.0)
        .add_field(node.1)
        .add_field(node.2)
        .build()
        .expect("a node is always exactly three fields");
    OwnedValue::try_from(Value::from(structure)).expect("a structure is a value")
}

/// The menu object a host reads and clicks.
pub struct MenuInterface {
    menu: Arc<Mutex<Menu>>,
    revision: Arc<Mutex<u32>>,
    /// Clicked actions, as the strings the [`Menu`] carries. Unbounded
    /// and never awaited on, for the same reason the item's clicks are:
    /// the host is blocked on `Event` returning.
    clicks: tokio::sync::mpsc::UnboundedSender<String>,
}

impl MenuInterface {
    pub fn new(
        menu: Arc<Mutex<Menu>>,
        revision: Arc<Mutex<u32>>,
        clicks: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Self {
        MenuInterface {
            menu,
            revision,
            clicks,
        }
    }
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl MenuInterface {
    /// `(revision, (id, props, children))`.
    ///
    /// `parent_id` 0 is the root, which is not a row of its own: it
    /// carries no label and exists only to hold the top-level items.
    async fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, LayoutNode) {
        let menu = self.menu.lock().await;
        let revision = *self.revision.lock().await;

        if parent_id == 0 {
            let mut root: HashMap<String, OwnedValue> = HashMap::new();
            root.insert(
                "children-display".to_string(),
                OwnedValue::try_from(Value::from("submenu")).expect("a string is a value"),
            );
            return (revision, node(0, root, &menu.items, recursion_depth));
        }

        match menu.find(parent_id) {
            Some(item) => (
                revision,
                node(item.id, properties(item), &item.children, recursion_depth),
            ),
            // An id we do not know is answered with an empty node rather
            // than an error: hosts ask about stale ids after a layout
            // change, and an error there shows the user a broken menu for
            // something that has merely moved.
            None => (revision, node(parent_id, HashMap::new(), &[], 0)),
        }
    }

    async fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let menu = self.menu.lock().await;
        menu.flatten()
            .into_iter()
            .filter(|item| ids.is_empty() || ids.contains(&item.id))
            .map(|item| (item.id, properties(item)))
            .collect()
    }

    async fn get_property(&self, id: i32, name: String) -> OwnedValue {
        let menu = self.menu.lock().await;
        menu.find(id)
            .and_then(|item| properties(item).remove(&name))
            .unwrap_or_else(|| {
                OwnedValue::try_from(Value::from("")).expect("a string is a value")
            })
    }

    /// A click, a hover, or an open. Only `clicked` does anything.
    async fn event(&self, id: i32, event_id: String, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let action = self.menu.lock().await.action_for(id).map(str::to_string);
        if let Some(action) = action {
            let _ = self.clicks.send(action);
        }
    }

    /// Hosts batch events. Answering only `Event` leaves menus that use
    /// this one entirely unclickable.
    async fn event_group(
        &self,
        events: Vec<(i32, String, Value<'_>, u32)>,
    ) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, event_id, data, timestamp).await;
        }
        Vec::new()
    }

    /// `false` — the layout this returns is always current, because the
    /// daemon rebuilds it on its own schedule and bumps the revision.
    /// Answering `true` makes a host re-fetch the whole tree on every
    /// open for no gain.
    async fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    async fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    #[zbus(property)]
    async fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    async fn status(&self) -> String {
        "normal".to_string()
    }

    #[zbus(property)]
    async fn text_direction(&self) -> String {
        "ltr".to_string()
    }

    /// Empty: icons are named and resolved from the user's own theme, the
    /// same rule the tray item follows.
    #[zbus(property)]
    async fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(signal)]
    pub async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::MenuItem;

    fn menu() -> Menu {
        Menu::new(vec![
            MenuItem::checkmark("Wi-Fi", true, "radio:toggle"),
            MenuItem::separator(),
            MenuItem::standard("home", "connect:home"),
        ])
    }

    /// A separator must say so. Without `type = separator` a host draws a
    /// clickable blank row where the rule should be.
    #[test]
    fn a_separator_is_marked_as_one_and_carries_nothing_else() {
        let menu = menu();
        let sep = menu.flatten().into_iter().find(|i| i.kind == ItemKind::Separator).unwrap();
        let props = properties(sep);
        assert_eq!(
            String::try_from(props["type"].clone()).unwrap(),
            "separator"
        );
        assert!(!props.contains_key("label"), "a separator has no label to draw");
    }

    /// A checkmark needs both keys. `toggle-state` alone draws nothing,
    /// because the host does not know it is a checkbox.
    #[test]
    fn a_checkmark_sends_both_the_type_and_the_state() {
        let menu = menu();
        let wifi = menu.flatten().into_iter().find(|i| i.label == "Wi-Fi").unwrap();
        let props = properties(wifi);
        assert_eq!(
            String::try_from(props["toggle-type"].clone()).unwrap(),
            "checkmark"
        );
        assert_eq!(i32::try_from(props["toggle-state"].clone()).unwrap(), 1);
    }

    #[test]
    fn an_ordinary_row_is_not_given_a_toggle_a_host_would_draw() {
        let menu = menu();
        let home = menu.flatten().into_iter().find(|i| i.label == "home").unwrap();
        let props = properties(home);
        assert!(!props.contains_key("toggle-type"));
        assert!(!props.contains_key("toggle-state"));
    }

    /// The shape the whole protocol turns on. A node is
    /// `(id, a{sv}, av)`, and the children are variants each wrapping
    /// another node — get this wrong and the host shows an empty popup
    /// with nothing logged at either end.
    #[test]
    fn a_layout_node_has_three_fields_and_its_children_are_nested_nodes() {
        let menu = menu();
        let root = node(0, HashMap::new(), &menu.items, -1);
        assert_eq!(root.0, 0, "the root is id 0");
        assert_eq!(root.2.len(), 3, "three top-level rows");

        // Each child must be a variant wrapping another node of the same
        // shape — that recursion is the whole protocol.
        let first = root.2.first().expect("a child");
        let Value::Structure(child) = Value::from(first.clone()) else {
            panic!("each child is a variant wrapping a structure");
        };
        assert_eq!(
            child.fields().len(),
            3,
            "a child is a node of the same (id, properties, children) shape"
        );
    }

    /// `recursion_depth = 0` means one level only. A host asking for one
    /// level and receiving the whole tree is merely wasteful; a host
    /// asking for everything and receiving one level shows empty
    /// submenus.
    #[test]
    fn depth_zero_returns_the_node_without_its_children() {
        let menu = Menu::new(vec![MenuItem {
            children: vec![MenuItem::standard("child", "c")],
            ..MenuItem::standard("parent", "p")
        }]);
        let shallow = node(0, HashMap::new(), &menu.items, 0);
        assert!(shallow.2.is_empty(), "depth 0 stops at this node");
    }

    /// The signature hosts parse against. A wrong one here is the failure
    /// this whole module's comment warns about: an empty popup, logged
    /// nowhere.
    #[test]
    fn the_layout_node_signature_is_the_one_the_protocol_specifies() {
        assert_eq!(LayoutNode::SIGNATURE.to_string(), "(ia{sv}av)");
    }
}
