use crate::{Field, FromSql, IsNull, Kind, Type};
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
        reader: &mut CompositeReader<'_, 'a>,
    ) -> Result<Self, Box<dyn Error + Sync + Send>>;
}

pub struct CompositeReader<'f, 'a> {
    fields: &'f [Field],
    raw: &'a [u8],
    index: usize,
    rest: &'a [u8],
}

impl<'f, 'a> CompositeReader<'f, 'a> {
    fn new(fields: &'f [Field], raw: &'a [u8]) -> CompositeReader<'f, 'a> {
        CompositeReader {
            fields,
            raw,
            index: 0,
            rest: raw,
        }
    }

    pub fn get<T>(&mut self, name: &str) -> Result<T, Box<dyn Error + Sync + Send>>
    where
        T: FromSql<'a>,
    {
        for _ in 0..self.fields.len() {
            if self.index == self.fields.len() {
                self.index = 0;
                self.rest = self.raw;
            }

            let field = &self.fields[self.index];
            let oid = read_be_i32(&mut self.rest)? as u32;
            if oid != field.type_().oid() {
                return Err("unexpected OID".into());
            }
            let value = read_raw_value(&mut self.rest)?;
            self.index += 1;

            if field.name() == name {
                return T::from_sql_nullable(field.type_(), value);
            }
        }

        Err("composite type is missing a field".into())
    }
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
