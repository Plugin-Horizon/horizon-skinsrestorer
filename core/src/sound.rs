//! The sound a skin change plays to the player (`server.sound.value`), parsed as the original's
//! `SoundParser` does.

/// A sound event by name, for a Sound packet with an inline event.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    /// `minecraft:entity.player.teleport`.
    pub id: String,
    /// The sound category in the engine's spelling (`master`, `players`, ...).
    pub category: String,
    pub volume: f32,
    pub pitch: f32,
}

/// Bukkit's `SoundCategory` name as the engine's `SoundSource` (None: no such category, which
/// the original's `Category.valueOf` rejects).
fn category_of(name: &str) -> Option<String> {
    Some(
        match name.to_ascii_lowercase().as_str() {
            c @ ("master" | "music" | "weather" | "hostile" | "neutral" | "ambient" | "voice" | "ui") => c,
            "records" => "record",
            "blocks" => "block",
            "players" => "player",
            _ => return None,
        }
        .to_owned(),
    )
}

/// `SoundParser.parse`: spaces removed, split at `,` (empty parts dropped); the first part is
/// `[~][category@]name`, then volume, pitch and a seed (the seed is read and unused). Empty or
/// `none` plays nothing; an invalid number makes the whole value invalid (the original logs
/// "Invalid sound value in config" and plays nothing).
///
/// A name is a Bukkit `Sound` constant (`ENTITY_PLAYER_TELEPORT`: lower case, `_` as `.`;
/// constants whose vanilla id has an underscore need the id) or a vanilla id
/// (`minecraft:entity.player.teleport`).
pub fn parse(value: &str) -> Option<Sound> {
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return None;
    }
    let compact: String = value.chars().filter(|c| *c != ' ').collect();
    let parts: Vec<&str> = compact.split(',').filter(|p| !p.is_empty()).collect();
    let first = *parts.first()?;
    let first = first.strip_prefix('~').unwrap_or(first);
    let (category, name) = match first.split_once('@') {
        Some((c, n)) => (category_of(c)?, n),
        None => ("master".to_string(), first),
    };
    if name.is_empty() {
        return None;
    }
    let number = |i: usize, default: f32| -> Option<f32> {
        parts.get(i).map_or(Some(default), |p| p.parse().ok())
    };
    let volume = number(1, 1.0)?;
    let pitch = number(2, 1.0)?;
    if let Some(seed) = parts.get(3) {
        seed.parse::<i64>().ok()?;
    }
    let lower = name.to_ascii_lowercase();
    let id = if name.contains(':') {
        lower
    } else if name.contains('.') {
        format!("minecraft:{lower}")
    } else {
        format!("minecraft:{}", lower.replace('_', "."))
    };
    Some(Sound {
        id,
        category,
        volume,
        pitch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_default() {
        // The second value is the pitch only after the volume: "name, volume, pitch".
        let s = parse("ENTITY_PLAYER_TELEPORT, 0.7").unwrap();
        assert_eq!(s.id, "minecraft:entity.player.teleport");
        assert_eq!(s.category, "master");
        assert_eq!((s.volume, s.pitch), (0.7, 1.0));
    }

    #[test]
    fn parses_ids_category_and_three_parts() {
        let s = parse("players@minecraft:block.note_block.pling, 0.5, 2").unwrap();
        assert_eq!(s.id, "minecraft:block.note_block.pling");
        assert_eq!(s.category, "player");
        assert_eq!((s.volume, s.pitch), (0.5, 2.0));
        assert_eq!(parse("~entity.player.levelup").unwrap().id, "minecraft:entity.player.levelup");
    }

    #[test]
    fn nothing_for_none_empty_or_bad_numbers() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("NONE"), None);
        assert_eq!(parse("ENTITY_PLAYER_TELEPORT, loud"), None);
        assert_eq!(parse("@"), None);
        assert_eq!(parse("nowhere@entity.player.levelup"), None);
    }
}
