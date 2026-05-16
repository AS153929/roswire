#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingRequest {
    pub tokens: Vec<String>,
}

impl MappingRequest {
    pub fn new(tokens: Vec<String>) -> Self {
        Self { tokens }
    }
}

#[cfg(test)]
mod tests {
    use super::MappingRequest;

    #[test]
    fn new_keeps_all_tokens() {
        let request = MappingRequest::new(vec!["ip".into(), "address".into(), "print".into()]);
        assert_eq!(request.tokens, vec!["ip", "address", "print"]);
    }
}
