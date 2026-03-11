// @generated automatically by Diesel CLI.

diesel::table! {
    users (id) {
        id -> Integer,
        name -> Text,
        #[sql_name = "type"]
        type_ -> Text,
        password -> Text,
        token -> Nullable<Text>,
    }
}
