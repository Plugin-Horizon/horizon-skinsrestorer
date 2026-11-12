//! The skins menu, as the proxy sends it (`openGUI`) and as a click answers it (`guiActionList`).
//!
//! The menu is the original's `SRInventory` codec: gzip over the rest of the message; rows (int),
//! title (JSON text), then the slots as a count and (slot int, item). An item is a material name, a
//! display name (JSON), lore (JSON lines), an optional skull texture hash, the glint flag, and its
//! click handlers (a count and (click type, actions, close flag)). Every string is `writeUTF`.
//!
//! Engine-free: [`Inventory`] and [`MenuItem`] are what the glue turns into a native menu, and
//! [`encode_actions`] is the reply the proxy reads.
use crate::payload::{encode_modified_utf8, Reader};
use crate::skins;
use std::io::Read;

/// The most a menu's bytes may inflate to (six rows of slots and their text is a few KiB).
const INFLATE_LIMIT: u64 = 1 << 20;

/// A skin named in an action: `SkinIdentifier` (identifier, optional variant, type name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkinRef {
    pub identifier: String,
    pub variant: Option<String>,
    pub kind: String,
}

/// One of the proxy's click actions (`GUIActionChannelPayload`). `Unknown` is an action type this
/// version does not know: it is kept in the list and not sent back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuiAction {
    OpenPage { page: i32, page_type: String },
    ClearSkin,
    SetSkin(SkinRef),
    AddFavourite(SkinRef),
    RemoveFavourite(SkinRef),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClickAction {
    pub actions: Vec<GuiAction>,
    pub close: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The material's name (`MaterialType`, lower case): dirt, skull, arrow, ...
    pub material: String,
    /// The display name, as JSON text.
    pub name: String,
    /// The lore lines, as JSON text.
    pub lore: Vec<String>,
    pub texture_hash: Option<String>,
    pub glint: bool,
    /// The click type (left, right, shift_left, other) and what it does.
    pub handlers: Vec<(String, ClickAction)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    pub rows: u8,
    /// The title, as JSON text.
    pub title: String,
    /// Slot and item, in the order the proxy sent them.
    pub items: Vec<(usize, Item)>,
}

/// An item as the glue builds its engine stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// The engine's item id, e.g. `minecraft:player_head`.
    pub material: &'static str,
    /// The custom name, as JSON text.
    pub name: String,
    pub lore: Vec<String>,
    /// The base64 `textures` value of a head (see [`skins::texture_value`]).
    pub texture_value: Option<String>,
    pub glint: bool,
}

/// `MaterialType` to the item the original gives (`BukkitGUI` / `XMaterial`); an unknown name is
/// dirt, the codec's default.
pub fn material_id(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "skull" => "minecraft:player_head",
        "arrow" => "minecraft:arrow",
        "barrier" => "minecraft:barrier",
        "bookshelf" => "minecraft:bookshelf",
        "ender_eye" => "minecraft:ender_eye",
        "enchanting_table" => "minecraft:enchanting_table",
        _ => "minecraft:dirt",
    }
}

/// The engine item of a slot. A head gets its texture only when it is a skull (Bukkit's
/// `SkullUtil` runs for the skull type alone).
pub fn menu_item(item: &Item) -> MenuItem {
    let material = material_id(&item.material);
    let texture_value = match (&item.texture_hash, material) {
        (Some(hash), "minecraft:player_head") => Some(skins::texture_value(hash)),
        _ => None,
    };
    MenuItem {
        material,
        name: item.name.clone(),
        lore: item.lore.clone(),
        texture_value,
        glint: item.glint,
    }
}

/// The click type of a click (`ModGUI`): a plain click's button 0 is left, 1 right; a shift-click
/// (mode 1, quick move) with button 0 is shift-left; the rest is other.
pub fn click_type(button: i32, mode: i32) -> &'static str {
    match (mode, button) {
        (0, 0) => "left",
        (0, 1) => "right",
        (1, 0) => "shift_left",
        _ => "other",
    }
}

/// The menu of an `openGUI` message (`data`, the gzip stream), or None when it is not one.
pub fn decode(data: &[u8]) -> Option<Inventory> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(data)
        .take(INFLATE_LIMIT)
        .read_to_end(&mut raw)
        .ok()?;
    let mut r = Reader::new(&raw);
    let rows = u8::try_from(r.i32()?).ok().filter(|n| (1..=6).contains(n))?;
    let title = r.string()?;
    let count = r.i32()?;
    let mut items = Vec::new();
    for _ in 0..count.max(0) {
        let slot = usize::try_from(r.i32()?).ok()?;
        let item = read_item(&mut r)?;
        if slot < usize::from(rows) * 9 {
            items.push((slot, item));
        }
    }
    Some(Inventory { rows, title, items })
}

fn read_item(r: &mut Reader) -> Option<Item> {
    let material = r.string()?;
    let name = r.string()?;
    let lore_count = r.i32()?;
    let mut lore = Vec::new();
    for _ in 0..lore_count.max(0) {
        lore.push(r.string()?);
    }
    let texture_hash = if r.boolean()? { Some(r.string()?) } else { None };
    let glint = r.boolean()?;
    let handler_count = r.i32()?;
    let mut handlers = Vec::new();
    for _ in 0..handler_count.max(0) {
        let kind = r.string()?;
        let action_count = r.i32()?;
        let mut actions = Vec::new();
        for _ in 0..action_count.max(0) {
            actions.push(read_action(r)?);
        }
        let close = r.boolean()?;
        handlers.push((kind, ClickAction { actions, close }));
    }
    Some(Item {
        material,
        name,
        lore,
        texture_hash,
        glint,
        handlers,
    })
}

/// `GUIActionChannelPayload`: the type's name, then that type's fields.
fn read_action(r: &mut Reader) -> Option<GuiAction> {
    Some(match r.string()?.as_str() {
        "openPage" => GuiAction::OpenPage {
            page: r.i32()?,
            page_type: r.string()?,
        },
        "clearSkin" => GuiAction::ClearSkin,
        "setSkin" => GuiAction::SetSkin(read_skin(r)?),
        "addFavourite" => GuiAction::AddFavourite(read_skin(r)?),
        "removeFavourite" => GuiAction::RemoveFavourite(read_skin(r)?),
        _ => GuiAction::Unknown,
    })
}

fn read_skin(r: &mut Reader) -> Option<SkinRef> {
    let identifier = r.string()?;
    let variant = if r.boolean()? { Some(r.string()?) } else { None };
    let kind = r.string()?;
    Some(SkinRef {
        identifier,
        variant,
        kind,
    })
}

/// The click type that a click handler has, from its name in the menu (`left`, `right`, ...).
pub fn handler<'a>(item: &'a Item, click: &str) -> Option<&'a ClickAction> {
    item.handlers.iter().find(|(k, _)| k == click).map(|(_, a)| a)
}

/// The proxy's reply to a click (`guiActionList`): the channel name, then the actions. `Unknown`
/// actions are left out, as the proxy would not know them either.
pub fn encode_actions(actions: &[GuiAction]) -> Option<Vec<u8>> {
    let known: Vec<&GuiAction> = actions.iter().filter(|a| **a != GuiAction::Unknown).collect();
    let mut out = encode_modified_utf8("guiActionList")?;
    out.extend((known.len() as i32).to_be_bytes());
    for a in known {
        match a {
            GuiAction::OpenPage { page, page_type } => {
                out.extend(encode_modified_utf8("openPage")?);
                out.extend(page.to_be_bytes());
                out.extend(encode_modified_utf8(page_type)?);
            }
            GuiAction::ClearSkin => out.extend(encode_modified_utf8("clearSkin")?),
            GuiAction::SetSkin(s) => {
                out.extend(encode_modified_utf8("setSkin")?);
                write_skin(&mut out, s)?;
            }
            GuiAction::AddFavourite(s) => {
                out.extend(encode_modified_utf8("addFavourite")?);
                write_skin(&mut out, s)?;
            }
            GuiAction::RemoveFavourite(s) => {
                out.extend(encode_modified_utf8("removeFavourite")?);
                write_skin(&mut out, s)?;
            }
            GuiAction::Unknown => {}
        }
    }
    Some(out)
}

fn write_skin(out: &mut Vec<u8>, s: &SkinRef) -> Option<()> {
    out.extend(encode_modified_utf8(&s.identifier)?);
    match &s.variant {
        Some(v) => {
            out.push(1);
            out.extend(encode_modified_utf8(v)?);
        }
        None => out.push(0),
    }
    out.extend(encode_modified_utf8(&s.kind)?);
    Some(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    fn utf(s: &str) -> Vec<u8> {
        encode_modified_utf8(s).unwrap()
    }

    /// A menu as the original writes it, gzipped: one skull with a click handler and one dirt block.
    pub fn menu_bytes(rows: i32) -> Vec<u8> {
        let mut raw = Vec::new();
        raw.extend(rows.to_be_bytes());
        raw.extend(utf("{\"text\":\"Skins\"}"));
        raw.extend(2i32.to_be_bytes());
        // Slot 10: a skull showing a texture, left click opens page 2 of the main menu, and closes.
        raw.extend(10i32.to_be_bytes());
        raw.extend(utf("skull"));
        raw.extend(utf("{\"text\":\"Knight\"}"));
        raw.extend(1i32.to_be_bytes());
        raw.extend(utf("{\"text\":\"lore\"}"));
        raw.push(1);
        raw.extend(utf("tex123"));
        raw.push(0);
        raw.extend(1i32.to_be_bytes());
        raw.extend(utf("left"));
        raw.extend(1i32.to_be_bytes());
        raw.extend(utf("openPage"));
        raw.extend(2i32.to_be_bytes());
        raw.extend(utf("main"));
        raw.push(1);
        // Slot 13: a dirt block with no handlers.
        raw.extend(13i32.to_be_bytes());
        raw.extend(utf("dirt"));
        raw.extend(utf("{\"text\":\"Plain\"}"));
        raw.extend(0i32.to_be_bytes());
        raw.push(0);
        raw.push(1);
        raw.extend(0i32.to_be_bytes());
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn a_menu_decodes_with_its_slots_and_handlers() {
        let inv = decode(&menu_bytes(3)).unwrap();
        assert_eq!(inv.rows, 3);
        assert_eq!(inv.title, "{\"text\":\"Skins\"}");
        assert_eq!(inv.items.len(), 2);
        let (slot, skull) = &inv.items[0];
        assert_eq!(*slot, 10);
        assert_eq!(skull.material, "skull");
        assert_eq!(skull.texture_hash.as_deref(), Some("tex123"));
        assert_eq!(skull.lore, vec!["{\"text\":\"lore\"}".to_owned()]);
        let left = handler(skull, "left").unwrap();
        assert!(left.close);
        assert_eq!(
            left.actions,
            vec![GuiAction::OpenPage { page: 2, page_type: "main".into() }]
        );
        assert!(handler(skull, "right").is_none());
        assert!(inv.items[1].1.texture_hash.is_none());
    }

    #[test]
    fn slots_past_the_rows_and_bad_rows_are_refused() {
        // One row holds slots 0..9: the fixture's slots 10 and 13 are outside it.
        assert_eq!(decode(&menu_bytes(1)).unwrap().items.len(), 0);
        // Two rows hold 0..18: both are inside.
        assert_eq!(decode(&menu_bytes(2)).unwrap().items.len(), 2);
        assert!(decode(&menu_bytes(0)).is_none());
        assert!(decode(&menu_bytes(7)).is_none());
        assert!(decode(b"not gzip").is_none());
    }

    #[test]
    fn the_menu_item_gets_a_head_only_for_a_skull() {
        let inv = decode(&menu_bytes(3)).unwrap();
        // The fixture's "skull" is the codec's SKULL, lower case: a head with its texture.
        let head = menu_item(&inv.items[0].1);
        assert_eq!(head.material, "minecraft:player_head");
        assert_eq!(head.texture_value, Some(skins::texture_value("tex123")));
        // Dirt never gets a texture, even when it has a hash.
        let mut dirt = inv.items[1].1.clone();
        dirt.texture_hash = Some("tex999".into());
        let dirt = menu_item(&dirt);
        assert_eq!(dirt.material, "minecraft:dirt");
        assert_eq!(dirt.texture_value, None);
        assert!(dirt.glint);
    }

    #[test]
    fn materials_and_clicks_map_as_the_original_does() {
        assert_eq!(material_id("ENCHANTING_TABLE"), "minecraft:enchanting_table");
        assert_eq!(material_id("ender_eye"), "minecraft:ender_eye");
        assert_eq!(material_id("nothing"), "minecraft:dirt");
        // click_type(button, mode): a plain click is mode 0, a shift-click (quick move) mode 1.
        assert_eq!(click_type(0, 0), "left");
        assert_eq!(click_type(1, 0), "right");
        assert_eq!(click_type(2, 0), "other", "a middle-click is not a left click");
        assert_eq!(click_type(0, 1), "shift_left");
        assert_eq!(click_type(1, 1), "other", "a right shift-click is other");
        assert_eq!(click_type(0, 6), "other");
    }

    #[test]
    fn the_reply_is_the_channel_name_then_the_actions() {
        let actions = vec![
            GuiAction::SetSkin(SkinRef {
                identifier: "knight".into(),
                variant: Some("slim".into()),
                kind: "custom".into(),
            }),
            GuiAction::Unknown,
            GuiAction::ClearSkin,
        ];
        let data = encode_actions(&actions).unwrap();
        let mut want = utf("guiActionList");
        want.extend(2i32.to_be_bytes());
        want.extend(utf("setSkin"));
        want.extend(utf("knight"));
        want.push(1);
        want.extend(utf("slim"));
        want.extend(utf("custom"));
        want.extend(utf("clearSkin"));
        assert_eq!(data, want);
    }
}
