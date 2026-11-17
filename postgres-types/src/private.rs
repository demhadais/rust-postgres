use crate::{Field, FromSql, IsNull, Type};
pub use bytes::BytesMut;
use std::error::Error;

pub fn read_be_i32(buf: &mut &[u8]) -> Result<i32, Box<dyn Error + Sync + Send>> {
    let val = buf
        .get(..4)
        .ok_or("invalid buffer size")?
        .try_into()
        .unwrap();
    *buf = &buf[4..];
    Ok(i32::from_be_bytes(val))
}

pub fn read_value<'a, T>(
    type_: &Type,
    buf: &mut &'a [u8],
) -> Result<T, Box<dyn Error + Sync + Send>>
where
    T: FromSql<'a>,
{
    let value = read_raw_value(buf)?;
    T::from_sql_nullable(type_, value)
}

pub fn read_field<'a, T>(
    fields: &[Field],
    mut buf: &'a [u8],
    name: &str,
) -> Result<T, Box<dyn Error + Sync + Send>>
where
    T: FromSql<'a>,
{
    for field in fields {
        let oid = read_be_i32(&mut buf)? as u32;
        if oid != field.type_().oid() {
            return Err("unexpected OID".into());
        }

        let value = read_raw_value(&mut buf)?;
        if field.name() == name {
            return T::from_sql_nullable(field.type_(), value);
        }
    }

    Err("composite type is missing a field".into())
}

fn read_raw_value<'a>(
    buf: &mut &'a [u8],
) -> Result<Option<&'a [u8]>, Box<dyn Error + Sync + Send>> {
    let len = read_be_i32(buf)?;
    if len < 0 {
        return Ok(None);
    }

    let (head, tail) = buf
        .split_at_checked(len as usize)
        .ok_or("invalid buffer size")?;
    *buf = tail;
    Ok(Some(head))
}

pub trait ToSqlComposite {
    const NUM_FIELDS: usize;

    fn accepts_field(field: &Field) -> bool;

    fn field_to_sql(
        &self,
        field: &Field,
        out: &mut BytesMut,
    ) -> Option<Result<IsNull, Box<dyn Error + Sync + Send>>>;
}

pub trait FromSqlComposite<'a>: Sized {
    const NUM_FIELDS: usize;

    fn accepts_field(field: &Field) -> bool;

    fn from_sql_fields(
        fields: &[Field],
        buf: &'a [u8],
    ) -> Result<Self, Box<dyn Error + Sync + Send>>;
}
