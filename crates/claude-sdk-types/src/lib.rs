//! Claude CLI stdin/stdout contracts derived from pinned Agent SDK declarations.
//!
//! This crate does not launch Claude, implement its agent loop, or handle login.
//! See `source-manifest.json` for protocol roots, coverage, and the source hash.

mod generated;
pub use generated::*;

/// An optional property. Unlike `Option`, this preserves absent versus JSON null.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Presence<T> {
    #[default]
    Missing,
    Present(T),
}

impl<T> Presence<T> {
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

impl<T: serde::Serialize> serde::Serialize for Presence<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Present(value) => value.serialize(serializer),
            Self::Missing => Err(serde::ser::Error::custom("absent property must be omitted")),
        }
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Presence<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::Present)
    }
}

fn required<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(deserializer)
}

macro_rules! literal_type {
    ($name:ident, $value:literal) => {
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct $name;

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serde::Serialize::serialize(&$value, serializer)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
                if value == serde_json::json!($value) {
                    Ok(Self)
                } else {
                    Err(serde::de::Error::custom(concat!(
                        "expected literal ",
                        stringify!($value)
                    )))
                }
            }
        }
    };
}
pub(crate) use literal_type;

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
