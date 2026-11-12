use crate::{
    CompositeReader, Field, FromSql, FromSqlComposite, IsNull, Kind, ToSqlComposite, Type,
};
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

pub fn composite_accepts(
    type_: &Type,
    name: &str,
    num_fields: usize,
    accepts_field: fn(&Field) -> bool,
) -> bool {
    if type_.name() != name {
        return false;
    }

    match type_.kind() {
        Kind::Composite(fields) => fields.len() == num_fields && fields.iter().all(accepts_field),
        _ => false,
    }
}

pub fn composite_to_sql<T>(
    value: &T,
    type_: &Type,
    buf: &mut BytesMut,
) -> Result<IsNull, Box<dyn Error + Sync + Send>>
where
    T: ToSqlComposite,
{
    let fields = match type_.kind() {
        Kind::Composite(fields) => fields,
        _ => unreachable!(),
    };

    buf.extend_from_slice(&(fields.len() as i32).to_be_bytes());

    for field in fields {
        buf.extend_from_slice(&field.type_().oid().to_be_bytes());

        let base = buf.len();
        buf.extend_from_slice(&[0; 4]);
        let r = value
            .field_to_sql(field, buf)
            .ok_or("composite type has an unexpected field")?;

        let count = match r? {
            IsNull::Yes => -1,
            IsNull::No => {
                let len = buf.len() - base - 4;
                if len > i32::MAX as usize {
                    return Err("value too large to transmit".into());
                }
                len as i32
            }
        };

        buf[base..base + 4].copy_from_slice(&count.to_be_bytes());
    }

    Ok(IsNull::No)
}

pub fn composite_from_sql<'a, T>(
    type_: &Type,
    mut buf: &'a [u8],
) -> Result<T, Box<dyn Error + Sync + Send>>
where
    T: FromSqlComposite<'a>,
{
    let fields = match type_.kind() {
        Kind::Composite(fields) => fields,
        _ => unreachable!(),
    };

    let num_fields = read_be_i32(&mut buf)?;
    if num_fields as usize != fields.len() {
        return Err(format!("invalid field count: {} vs {}", num_fields, fields.len()).into());
    }

    T::from_sql_fields(&mut CompositeReader::new(fields, buf))
}

pub(crate) fn read_raw_value<'a>(
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
