#[derive(Debug, Clone, Default)]
pub struct PaginatedResult<T, C> {
    pub items: Vec<T>,
    pub next_cursor: Option<C>,
    pub has_more: bool,
}

impl<T, C> PaginatedResult<T, C> {
    pub fn new(items: Vec<T>, next_cursor: Option<C>, has_more: bool) -> Self {
        Self {
            items,
            next_cursor,
            has_more,
        }
    }

    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
            has_more: false,
        }
    }
}
