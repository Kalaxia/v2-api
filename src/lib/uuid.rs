use std::{
    marker::PhantomData,
    hash::{Hasher, Hash},
    fmt::{self, Debug, Formatter},
};
use sqlx::{self, Type, Postgres, Database, decode::Decode, encode::Encode, value::HasRawValue};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct Uuid<T> {
    id: uuid::Uuid,
    #[serde(skip)]
    _phantom: PhantomData<T>,
}

// We need to implement Clone, Copy, PartialEq, Eq and Hash ourselves, because T is not constrained
// as it only used as a marker for Uuid to be bound to any type.

impl<T> Debug for Uuid<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        self.id.fmt(f)
    }
}

impl<T> Clone for Uuid<T> {
    fn clone(&self) -> Self {
        Uuid {
            id: self.id.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T> Copy for Uuid<T> {}

impl<T> PartialEq for Uuid<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
    }
}
impl<T> Eq for Uuid<T> { }

impl<T> Hash for Uuid<T> {
    fn hash<H>(&self, state: &mut H) where H: Hasher {
        self.id.hash(state)
    }
}

// Uuid can never be created from raw uuid::Uuid, this ensures we can never convert an Uuid<T> to
// an Uuid<U> by extraction.
impl<T> Uuid<T> {
    pub fn new() -> Self {
        Uuid {
            id: uuid::Uuid::new_v4(),
            _phantom: PhantomData,
        }
    }
}

impl<T> From<Uuid<T>> for uuid::Uuid {
    fn from(id: Uuid<T>) -> Self {
        id.id
    }
}

impl<'a, T> Type<Postgres> for Uuid<T> {
    fn type_info() -> <Postgres as Database>::TypeInfo {
        uuid::Uuid::type_info()
    }
}

impl<'a, T> Decode<'a, Postgres> for Uuid<T>
where Uuid<T> : 'a,
{
    fn decode(value: <Postgres as HasRawValue<'a>>::RawValue) -> Result<Self, sqlx::Error> {
        Ok(Uuid {
            id: uuid::Uuid::decode(value)?,
            _phantom: PhantomData,
        })
    }
}

impl<T> Encode<Postgres> for Uuid<T> {
    fn encode(&self, buf: &mut <Postgres as Database>::RawBuffer) {
        self.id.encode(buf)
    }
}
