use diesel::prelude::*;
use serde::{Deserialize, Serialize};

// 用于插入数据库的结构体（不含密码）

#[derive(Queryable, Selectable, Serialize, Deserialize, Debug)]
#[diesel(table_name = crate::schema::users)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct User {
    pub id: i32,
    pub name: String,
    pub type_: String,
    pub password: String,
    pub token: Option<String>,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = crate::schema::users)]
pub struct UserInsert {
    pub name: String,
    pub type_: String,
}

#[derive(Deserialize)]
pub struct NewUser {
    pub name: String,
    pub type_: String,
    pub password: String,
}
