use postgres_types::{FromSql, ToSql};

#[derive(ToSql, Debug)]
#[postgres(flatten)]
struct ToSqlFlattenStruct {
    a: i32,
}

#[derive(FromSql, Debug)]
#[postgres(flatten)]
struct FromSqlFlattenStruct {
    a: i32,
}

#[derive(ToSql, Debug)]
enum ToSqlFlattenVariant {
    #[postgres(flatten)]
    Foo,
}

#[derive(FromSql, Debug)]
enum FromSqlFlattenVariant {
    #[postgres(flatten)]
    Foo,
}

#[derive(ToSql, Debug)]
struct ToSqlFlattenName {
    #[postgres(flatten, name = "b")]
    a: i32,
}

#[derive(FromSql, Debug)]
struct FromSqlFlattenName {
    #[postgres(flatten, name = "b")]
    a: i32,
}

fn main() {}
