#[derive(Clone, Debug, PartialEq)]
pub enum Face {
    Default,
    Keyword,
    Type,
    String,
    Comment,
    Function,
    Builtin,
}
