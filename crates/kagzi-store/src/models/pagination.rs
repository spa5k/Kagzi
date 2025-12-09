#[derive(Debug, Clone)]
pub struct PaginatedResult<T, C> {
    pub items: Vec<T>,
    pub next_cursor: Option<C>,
    pub has_more: bool,
}

impl<T, C> PaginatedResult<T, C> {
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
            has_more: false,
        }
    }
}
