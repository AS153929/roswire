pub mod predicate {
    pub mod str {
        pub fn contains<P: Into<String>>(pattern: P) -> impl predicates::prelude::Predicate<str> {
            predicates::str::contains(pretty_json_fragment(pattern.into()))
        }

        #[allow(dead_code)]
        pub fn is_empty() -> impl predicates::prelude::Predicate<str> {
            predicates::str::is_empty()
        }

        fn pretty_json_fragment(pattern: String) -> String {
            pattern
                .replace("\":\"", "\": \"")
                .replace("\":[]", "\": []")
                .replace("\":false", "\": false")
                .replace("\":true", "\": true")
        }
    }
}
