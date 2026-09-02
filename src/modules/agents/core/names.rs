use std::time::{SystemTime, UNIX_EPOCH};

const ADJECTIVES: &[&str] = &[
    "Amber",
    "Ancient",
    "Arctic",
    "Autumn",
    "Azure",
    "Balanced",
    "Bold",
    "Brave",
    "Bright",
    "Brisk",
    "Calm",
    "Candid",
    "Cedar",
    "Celestial",
    "Clever",
    "Cloudy",
    "Copper",
    "Coral",
    "Cosmic",
    "Crimson",
    "Crystal",
    "Curious",
    "Daring",
    "Dawn",
    "Deep",
    "Desert",
    "Distant",
    "Dusky",
    "Early",
    "Eastern",
    "Electric",
    "Emerald",
    "Even",
    "Fabled",
    "Fair",
    "Fallen",
    "Fast",
    "Fern",
    "Fierce",
    "Floral",
    "Flying",
    "Forest",
    "Free",
    "Frosty",
    "Gentle",
    "Golden",
    "Grand",
    "Green",
    "Harbor",
    "Hidden",
    "High",
    "Hollow",
    "Indigo",
    "Iron",
    "Ivory",
    "Jade",
    "Keen",
    "Kind",
    "Lake",
    "Light",
    "Lively",
    "Lunar",
    "Maple",
    "Marine",
    "Mellow",
    "Misty",
    "Modern",
    "Moonlit",
    "Morning",
    "Mossy",
    "Mountain",
    "Nimble",
    "Noble",
    "Northern",
    "Nova",
    "Ocean",
    "Olive",
    "Open",
    "Orange",
    "Orchid",
    "Pacific",
    "Patient",
    "Pearl",
    "Pine",
    "Polar",
    "Prairie",
    "Prime",
    "Quiet",
    "Rapid",
    "Red",
    "River",
    "Royal",
    "Ruby",
    "Sage",
    "Sandy",
    "Scarlet",
    "Sea",
    "Serene",
    "Sharp",
    "Silver",
    "Sky",
    "Solar",
    "Southern",
    "Spring",
    "Still",
    "Stone",
    "Stormy",
    "Summer",
    "Sunny",
    "Swift",
    "Tall",
    "Teal",
    "Tender",
    "Tidal",
    "Umber",
    "Velvet",
    "Verdant",
    "Violet",
    "Warm",
    "Western",
    "Wild",
    "Windy",
    "Winter",
    "Wise",
    "Woodland",
    "Young",
    "Zenith",
    "Zesty",
];

const ANIMALS: &[&str] = &[
    "Albatross",
    "Antelope",
    "Badger",
    "Barracuda",
    "Bear",
    "Beaver",
    "Bison",
    "Bobcat",
    "Buffalo",
    "Butterfly",
    "Camel",
    "Caribou",
    "Cat",
    "Chamois",
    "Cheetah",
    "Cobra",
    "Condor",
    "Cormorant",
    "Cougar",
    "Coyote",
    "Crane",
    "Crow",
    "Deer",
    "Dingo",
    "Dolphin",
    "Dove",
    "Dragonfly",
    "Eagle",
    "Egret",
    "Elk",
    "Falcon",
    "Ferret",
    "Finch",
    "Fox",
    "Gazelle",
    "Gecko",
    "Gibbon",
    "Giraffe",
    "Goose",
    "Grouse",
    "Gull",
    "Hare",
    "Hawk",
    "Heron",
    "Ibex",
    "Ibis",
    "Jaguar",
    "Jay",
    "Kestrel",
    "Kingfisher",
    "Koala",
    "Lark",
    "Lemur",
    "Leopard",
    "Lion",
    "Lynx",
    "Magpie",
    "Marten",
    "Meerkat",
    "Mink",
    "Moose",
    "Moth",
    "Narwhal",
    "Newt",
    "Nightingale",
    "Ocelot",
    "Octopus",
    "Orca",
    "Oriole",
    "Osprey",
    "Otter",
    "Owl",
    "Panda",
    "Panther",
    "Parrot",
    "Peacock",
    "Pelican",
    "Penguin",
    "Pika",
    "Puffin",
    "Quail",
    "Rabbit",
    "Raccoon",
    "Raven",
    "Robin",
    "Salmon",
    "Sandpiper",
    "Seal",
    "Shark",
    "Skylark",
    "Sparrow",
    "Stag",
    "Starling",
    "Stingray",
    "Stork",
    "Swan",
    "Swift",
    "Tern",
    "Thrush",
    "Tiger",
    "Toucan",
    "Trout",
    "Turtle",
    "Viper",
    "Vulture",
    "Wallaby",
    "Walrus",
    "Weasel",
    "Whale",
    "Wildcat",
    "Wolf",
    "Wombat",
    "Wren",
    "Yak",
    "Zebra",
    "Bee",
    "Bittern",
    "Cardinal",
    "Chipmunk",
    "Curlew",
    "Firefly",
    "Flamingo",
    "Hedgehog",
    "Hummingbird",
    "Manatee",
    "Marmot",
    "Mongoose",
    "Plover",
];

pub(super) fn generated_name(mut occupied: impl FnMut(&str) -> bool) -> String {
    let combinations = ADJECTIVES.len().saturating_mul(ANIMALS.len());
    let seed = name_seed() as usize;
    for offset in 0..combinations {
        let index = seed.wrapping_add(offset) % combinations;
        let candidate = format!(
            "{}{}",
            ADJECTIVES[index / ANIMALS.len()],
            ANIMALS[index % ANIMALS.len()]
        );
        if !occupied(&candidate) {
            return candidate;
        }
    }
    loop {
        let candidate = format!("Worker{:x}", name_seed());
        if !occupied(&candidate) {
            return candidate;
        }
    }
}

fn name_seed() -> u64 {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64);
    mix(nanos ^ sequence.rotate_left(29))
}

const fn mix(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_have_a_large_namespace_and_valid_shape() {
        assert!(ADJECTIVES.len() * ANIMALS.len() >= 16_000);
        let name = generated_name(|_| false);
        assert!(crate::agents::valid_worker_name(&name));
        assert!(name.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }

    #[test]
    fn generation_skips_occupied_names() {
        let first = generated_name(|_| false);
        let second = generated_name(|candidate| candidate.eq_ignore_ascii_case(&first));
        assert_ne!(first, second);
    }

    #[test]
    fn worker_chosen_names_are_bounded_identifiers() {
        assert!(crate::agents::valid_worker_name("diff-review"));
        assert!(crate::agents::valid_worker_name("Auth_Tests2"));
        assert!(!crate::agents::valid_worker_name(""));
        assert!(!crate::agents::valid_worker_name("two words"));
        assert!(!crate::agents::valid_worker_name("-review"));
        assert!(!crate::agents::valid_worker_name(&"a".repeat(49)));
    }
}
