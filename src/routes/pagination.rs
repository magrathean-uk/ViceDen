use serde::Deserialize;

pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 100;
pub const MAX_SEARCH_QUERY_CHARS: usize = 160;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PageQuery {
    pub page: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageBounds {
    pub page: usize,
    pub limit: usize,
    pub offset: usize,
}

impl PageBounds {
    pub fn from_query(query: PageQuery) -> Self {
        let page = query.page.unwrap_or(1).max(1);
        let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let offset = page.saturating_sub(1).saturating_mul(limit);
        Self { page, limit, offset }
    }
}

pub fn bounded_search_query(raw: Option<&str>) -> String {
    raw.unwrap_or_default().trim().chars().take(MAX_SEARCH_QUERY_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_limit_and_page() {
        let bounds = PageBounds::from_query(PageQuery { page: Some(0), limit: Some(10_000) });
        assert_eq!(bounds.page, 1);
        assert_eq!(bounds.limit, MAX_LIMIT);
        assert_eq!(bounds.offset, 0);
    }
}
