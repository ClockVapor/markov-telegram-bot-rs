use std::collections::HashMap;

use log::error;
use rand::Rng;
use serde::{Deserialize, Serialize};
use substring::Substring;

use MarkovChainError::*;

pub type Counter = i64;

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkovChain {
    /// HashMap of word (#1) to HashMap of following word (#2) to number of times #2 has followed #1.
    pub data: HashMap<String, HashMap<String, Counter>>,
}

impl MarkovChain {
    /// Generates a Vec of words from the Markov chain. An optional seed word can be given to start with; otherwise,
    /// a weighted random one will be chosen based on starting words in the Markov chain.
    pub fn generate(&self, seed: Option<String>) -> Result<Vec<String>, MarkovChainError> {
        if self.data.is_empty() {
            return Err(Empty);
        }
        let mut word = match seed {
            // Use the given seed word
            Some(mut word) => {
                word = encode_db_field_name(&word);
                if !self.data.contains_key(&word) {
                    return Err(NoSuchSeed);
                }
                word
            }
            // Pick a random starting seed word
            None => match self.data.get("") {
                None => return Err(Empty),
                Some(starting_word_map) => choose_from_frequency_map(starting_word_map).clone(),
            },
        };

        let mut result: Vec<String> = vec![];
        while !word.is_empty() {
            result.push(decode_db_field_name(&word));
            match self.data.get(&word) {
                None => {
                    // Should never happen based on how we build the Markov chains
                    error!(
                        "Expected word {} to be in the Markov chain but it wasn't",
                        word
                    );
                    return Err(InternalError);
                }
                Some(word_map) => {
                    word = choose_from_frequency_map(word_map).clone();
                }
            }
        }

        Ok(result)
    }

    /// Adds each word pair in the given &str (separated by whitespace) to the Markov chain.
    pub fn add_message(&mut self, text: &str) {
        let mut words = text.split_whitespace().peekable();
        if words.peek().is_some() {
            let mut last_word = "";
            for word in words {
                self.add_word_pair(&last_word.to_owned(), &word.to_owned());
                last_word = word;
            }
            self.add_word_pair(&last_word.to_owned(), &"".to_owned());
        }
    }

    /// Removes a Markov chain's word pair counts from this Markov chain.
    pub fn remove_markov_chain(&mut self, other: &MarkovChain) {
        for (first, word_map) in other.data.iter() {
            for (second, counter) in word_map.iter() {
                self.remove_word_pair(first, second, counter);
            }
        }
    }

    /// Adds a single count to a pair of words in the Markov chain.
    fn add_word_pair(&mut self, first: &str, second: &str) {
        let first = encode_db_field_name(&first);

        match self.data.get_mut(first.as_str()) {
            Some(word_map) => match word_map.get(second) {
                Some(count) => {
                    let new_count = count + 1;
                    word_map.insert(second.to_string(), new_count);
                }
                None => {
                    word_map.insert(second.to_string(), 1);
                }
            },
            None => {
                let mut word_map = HashMap::new();
                word_map.insert(second.to_string(), 1);
                self.data.insert(first.to_string(), word_map);
            }
        }
    }

    /// Removes a given count from a pair of words in the Markov chain.
    fn remove_word_pair(&mut self, first: &str, second: &str, amount: &Counter) {
        let first = encode_db_field_name(&first);

        if let Some(word_map) = self.data.get_mut(first.as_str()) {
            if let Some(count) = word_map.get(second) {
                let new_count = count - amount;
                if new_count > 0 {
                    word_map.insert(second.to_string(), new_count);
                } else {
                    word_map.remove(second);
                    if word_map.is_empty() {
                        self.data.remove(first.as_str());
                    }
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TripletMarkovChain {
    /// HashMap of two words (#1 and #2) to HashMap of following word (#3) to number of times #3 has followed "#1 #2".
    pub data: HashMap<String, HashMap<String, Counter>>,
}

impl TripletMarkovChain {
    /// Generates a Vec of words from the Markov chain. An optional seed word can be given to start with; otherwise,
    /// a weighted random one will be chosen based on starting words in the Markov chain.
    pub fn generate(
        &self,
        seeds: Option<(String, String)>,
    ) -> Result<Vec<String>, MarkovChainError> {
        if self.data.is_empty() {
            return Err(Empty);
        }
        let mut pair = match seeds {
            // Use the given seed words
            Some(pair) => {
                if !self.data.contains_key(&pair_to_string(&pair)) {
                    return Err(NoSuchSeed);
                }
                pair
            }
            // Pick a random starting seed word
            None => match self
                .data
                .get(&pair_to_string(&("".to_string(), "".to_string())))
            {
                None => return Err(Empty),
                Some(starting_word_map) => (
                    "".to_string(),
                    choose_from_frequency_map(starting_word_map).clone(),
                ),
            },
        };

        let mut result: Vec<String> = vec![];
        if pair.0.as_str() != "" {
            result.push(pair.0.clone());
        }

        while pair.1.as_str() != "" {
            result.push(pair.1.clone());
            match self.data.get(&pair_to_string(&pair)) {
                None => {
                    // Should never happen based on how we build the Markov chains
                    error!(
                        "Expected pair ({}, {}) to be in the Markov chain but it wasn't",
                        pair.0, pair.1
                    );
                    return Err(InternalError);
                }
                Some(word_map) => {
                    pair = (pair.1, choose_from_frequency_map(word_map).clone());
                }
            }
        }

        Ok(result)
    }

    /// Adds each word pair in the given &str (separated by whitespace) to the Markov chain.
    pub fn add_message(&mut self, text: &str) {
        let mut words = text.split_whitespace().peekable();
        if words.peek().is_some() {
            let mut last_pair = ("".to_string(), "".to_string());
            for word in words {
                self.add_word_triplet(last_pair.clone(), word.to_string());
                last_pair = (last_pair.1, word.to_string());
            }
            self.add_word_triplet(last_pair.clone(), "".to_string());
            last_pair = (last_pair.1, "".to_string());
            self.add_word_triplet(last_pair, "".to_string());
        }
    }

    /// Removes a Markov chain's word triplet counts from this Markov chain.
    pub fn remove_markov_chain(&mut self, other: &TripletMarkovChain) {
        for (pair, word_map) in other.data.iter() {
            for (third, counter) in word_map.iter() {
                self.remove_word_triplet(&string_to_pair(pair), third, counter);
            }
        }
    }

    /// Adds a single count to a triplet of words in the Markov chain (i.e. The pair (first, second) is followed by
    /// third one more time).
    fn add_word_triplet(&mut self, pair: (String, String), third: String) {
        let pair_string = pair_to_string(&pair);
        match self.data.get_mut(&pair_string) {
            Some(word_map) => match word_map.get(third.as_str()) {
                Some(count) => {
                    let new_count = count + 1;
                    word_map.insert(third, new_count);
                }
                None => {
                    word_map.insert(third, 1);
                }
            },
            None => {
                let mut word_map = HashMap::new();
                word_map.insert(third, 1);
                self.data.insert(pair_string, word_map);
            }
        }
    }

    /// Removes a given count from a triplet of words in the Markov chain.
    fn remove_word_triplet(&mut self, pair: &(String, String), third: &str, amount: &Counter) {
        let pair_string = pair_to_string(pair);
        if let Some(word_map) = self.data.get_mut(&pair_string) {
            if let Some(count) = word_map.get(third) {
                let new_count = count - amount;
                if new_count > 0 {
                    word_map.insert(third.to_string(), new_count);
                } else {
                    word_map.remove(third);
                    if word_map.is_empty() {
                        self.data.remove(&pair_string);
                    }
                }
            }
        }
    }
}

/// Enum type for all errors that can arise during a Markov chain operation.
#[derive(Debug)]
pub enum MarkovChainError {
    /// A generating operation was attempted on an empty Markov chain.
    Empty,

    /// A seed was given for a Markov chain operation, but the Markov chain doesn't contain the seed.
    NoSuchSeed,

    /// Catch-all for unexpected Markov chain errors.
    InternalError,
}

/// Chooses a weighted random item from a frequency map.
fn choose_from_frequency_map<T>(map: &HashMap<T, Counter>) -> &T {
    // Build a cumulative distribution of each item
    let mut cumulative_distribution: Vec<(&T, Counter)> = vec![];
    let mut running_total: Counter = 0;
    for (item, count) in map {
        running_total += count;
        cumulative_distribution.push((item, running_total));
    }

    // Pick a random number, and see which item's "bucket" it lands in
    let random = rand::thread_rng().gen_range(0..running_total);
    for (item, cumulative_value) in cumulative_distribution {
        if random < cumulative_value {
            return item;
        }
    }
    panic!("Failed to choose random item from cumulative distribution");
}

/// Given a pair of two strings, returns a string of the two words with a single space between them.
fn pair_to_string(pair: &(String, String)) -> String {
    debug_assert!(
        !pair.0.chars().any(|c| c.is_whitespace()),
        "pair_to_string() was given a string containing whitespace: \"{}\"",
        pair.0
    );
    debug_assert!(
        !pair.1.chars().any(|c| c.is_whitespace()),
        "pair_to_string() was given a string containing whitespace: \"{}\"",
        pair.1
    );

    let mut result = pair.0.clone();
    result.push_str(" ");
    result.push_str(pair.1.as_str());
    encode_db_field_name(&result)
}

/// Given a string containing two words separated by a single space, returns a pair of the two separate words.
fn string_to_pair(s: &str) -> (String, String) {
    debug_assert!(!s.is_empty(), "string_to_pair() was given an empty string");
    debug_assert!(
        !s.chars().all(|c| c.is_whitespace()),
        "string_to_pair() was given a blank string"
    );

    match s.find(" ") {
        None => panic!("Invalid string given; contains no space: {}", s),
        Some(i) => (
            s.substring(0, i).to_string(),
            s.substring(i + 1, s.len()).to_string(),
        ),
    }
}

/// MongoDB 4 doesn't let field names start with '$'.
fn encode_db_field_name(s: &str) -> String {
    if s.starts_with("$") {
        let mut result = s.to_string();
        result.insert(0, '\\');
        result
    } else {
        s.to_string()
    }
}

/// MongoDB 4 doesn't let field names start with '$'.
fn decode_db_field_name(s: &str) -> String {
    if s.starts_with("\\$") {
        s.substring(1, s.len()).to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_to_pair() {
        assert_eq!(
            ("one".to_string(), "two".to_string()),
            string_to_pair("one two")
        );
    }

    #[test]
    fn test_pair_to_string() {
        assert_eq!(
            "one two",
            pair_to_string(&("one".to_string(), "two".to_string()))
        );
    }

    #[test]
    fn test_encode_db_field_name() {
        assert_eq!("foo", encode_db_field_name("foo"));
    }

    #[test]
    fn test_encode_db_field_name_leading_dollar_sign() {
        assert_eq!("\\$foo", encode_db_field_name("$foo"));
    }

    #[test]
    fn test_decode_db_field_name() {
        assert_eq!("foo", decode_db_field_name("foo"));
    }

    #[test]
    fn test_decode_db_field_name_leading_dollar_sign() {
        assert_eq!("$foo", decode_db_field_name("\\$foo"));
    }

    mod markov_chain {
        use super::*;

        #[test]
        fn test_add_word_pair() {
            let mut markov_chain = MarkovChain::default();
            assert_eq!(HashMap::default(), markov_chain.data);

            markov_chain.add_word_pair("one", "two");
            assert_eq!(
                HashMap::from([("two".to_string(), 1 as Counter)]),
                *markov_chain.data.get("one").unwrap()
            );

            markov_chain.add_word_pair("one", "two");
            assert_eq!(
                HashMap::from([("two".to_string(), 2 as Counter)]),
                *markov_chain.data.get("one").unwrap()
            );
        }

        #[test]
        fn test_add_word_pair_leading_dollar_sign() {
            let mut markov_chain = MarkovChain::default();
            assert_eq!(HashMap::default(), markov_chain.data);

            markov_chain.add_word_pair("$one", "two");
            assert_eq!(
                HashMap::from([("two".to_string(), 1 as Counter)]),
                *markov_chain.data.get("\\$one").unwrap()
            );

            markov_chain.add_word_pair("$one", "two");
            assert_eq!(
                HashMap::from([("two".to_string(), 2 as Counter)]),
                *markov_chain.data.get("\\$one").unwrap()
            );
        }

        #[test]
        fn test_remove_word_pair() {
            let mut markov_chain = MarkovChain {
                data: HashMap::from([(
                    "one".to_string(),
                    HashMap::from([("two".to_string(), 3 as Counter)]),
                )]),
            };

            markov_chain.remove_word_pair("one", "two", &(2 as Counter));
            assert_eq!(
                HashMap::from([("two".to_string(), 1 as Counter)]),
                *markov_chain.data.get("one").unwrap()
            );

            markov_chain.remove_word_pair("one", "two", &(1 as Counter));
            assert!(markov_chain.data.get("one").is_none());
        }

        #[test]
        fn test_remove_word_pair_leading_dollar_sign() {
            let mut markov_chain = MarkovChain {
                data: HashMap::from([(
                    "\\$one".to_string(),
                    HashMap::from([("two".to_string(), 3 as Counter)]),
                )]),
            };

            markov_chain.remove_word_pair("$one", "two", &(2 as Counter));
            assert_eq!(
                HashMap::from([("two".to_string(), 1 as Counter)]),
                *markov_chain.data.get("\\$one").unwrap()
            );

            markov_chain.remove_word_pair("$one", "two", &(1 as Counter));
            assert!(markov_chain.data.get("\\$one").is_none());
        }

        #[test]
        fn test_add_message() {
            let mut markov_chain = MarkovChain::default();
            assert_eq!(HashMap::default(), markov_chain.data);

            markov_chain.add_message("one two three");
            assert_eq!(
                HashMap::from([
                    (
                        "".to_string(),
                        HashMap::from([("one".to_string(), 1 as Counter)])
                    ),
                    (
                        "one".to_string(),
                        HashMap::from([("two".to_string(), 1 as Counter)])
                    ),
                    (
                        "two".to_string(),
                        HashMap::from([("three".to_string(), 1 as Counter)])
                    ),
                    (
                        "three".to_string(),
                        HashMap::from([("".to_string(), 1 as Counter)])
                    ),
                ]),
                markov_chain.data
            );
        }

        #[test]
        fn test_remove_markov_chain() {
            let mut markov_chain = MarkovChain {
                data: HashMap::from([(
                    "one".to_string(),
                    HashMap::from([("two".to_string(), 5 as Counter)]),
                )]),
            };
            let other = MarkovChain {
                data: HashMap::from([(
                    "one".to_string(),
                    HashMap::from([("two".to_string(), 2 as Counter)]),
                )]),
            };
            markov_chain.remove_markov_chain(&other);

            let expected = MarkovChain {
                data: HashMap::from([(
                    "one".to_string(),
                    HashMap::from([("two".to_string(), 3 as Counter)]),
                )]),
            };
            assert_eq!(expected, markov_chain);
        }
    }

    mod triplet_markov_chain {
        use super::*;

        #[test]
        fn test_add_word_triplet() {
            let mut markov_chain = TripletMarkovChain::default();
            assert_eq!(HashMap::default(), markov_chain.data);

            markov_chain
                .add_word_triplet(("one".to_string(), "two".to_string()), "three".to_string());
            assert_eq!(
                HashMap::from([("three".to_string(), 1 as Counter)]),
                *markov_chain.data.get("one two").unwrap()
            );

            markov_chain
                .add_word_triplet(("one".to_string(), "two".to_string()), "three".to_string());
            assert_eq!(
                HashMap::from([("three".to_string(), 2 as Counter)]),
                *markov_chain.data.get("one two").unwrap()
            );
        }

        #[test]
        fn test_add_word_triplet_leading_dollar_sign() {
            let mut markov_chain = TripletMarkovChain::default();
            assert_eq!(HashMap::default(), markov_chain.data);

            markov_chain
                .add_word_triplet(("$one".to_string(), "two".to_string()), "three".to_string());
            assert_eq!(
                HashMap::from([("three".to_string(), 1 as Counter)]),
                *markov_chain.data.get("\\$one two").unwrap()
            );

            markov_chain
                .add_word_triplet(("$one".to_string(), "two".to_string()), "three".to_string());
            assert_eq!(
                HashMap::from([("three".to_string(), 2 as Counter)]),
                *markov_chain.data.get("\\$one two").unwrap()
            );
        }

        #[test]
        fn test_remove_word_triplet() {
            let mut markov_chain = TripletMarkovChain {
                data: HashMap::from([(
                    "one two".to_string(),
                    HashMap::from([("three".to_string(), 3 as Counter)]),
                )]),
            };

            markov_chain.remove_word_triplet(
                &("one".to_string(), "two".to_string()),
                "three",
                &(2 as Counter),
            );
            assert_eq!(
                HashMap::from([("three".to_string(), 1 as Counter)]),
                *markov_chain.data.get("one two").unwrap()
            );

            markov_chain.remove_word_triplet(
                &("one".to_string(), "two".to_string()),
                "three",
                &(1 as Counter),
            );
            assert!(markov_chain.data.get("one two").is_none());
        }

        #[test]
        fn test_remove_word_triplet_leading_dollar_sign() {
            let mut markov_chain = TripletMarkovChain {
                data: HashMap::from([(
                    "\\$one two".to_string(),
                    HashMap::from([("three".to_string(), 3 as Counter)]),
                )]),
            };

            markov_chain.remove_word_triplet(
                &("$one".to_string(), "two".to_string()),
                "three",
                &(2 as Counter),
            );
            assert_eq!(
                HashMap::from([("three".to_string(), 1 as Counter)]),
                *markov_chain.data.get("\\$one two").unwrap()
            );

            markov_chain.remove_word_triplet(
                &("$one".to_string(), "two".to_string()),
                "three",
                &(1 as Counter),
            );
            assert!(markov_chain.data.get("\\$one two").is_none());
        }

        #[test]
        fn test_add_message() {
            let mut markov_chain = TripletMarkovChain::default();
            assert_eq!(HashMap::default(), markov_chain.data);

            markov_chain.add_message("one two three");
            assert_eq!(
                HashMap::from([
                    (
                        " ".to_string(),
                        HashMap::from([("one".to_string(), 1 as Counter)])
                    ),
                    (
                        " one".to_string(),
                        HashMap::from([("two".to_string(), 1 as Counter)])
                    ),
                    (
                        "one two".to_string(),
                        HashMap::from([("three".to_string(), 1 as Counter)])
                    ),
                    (
                        "two three".to_string(),
                        HashMap::from([("".to_string(), 1 as Counter)])
                    ),
                    (
                        "three ".to_string(),
                        HashMap::from([("".to_string(), 1 as Counter)])
                    ),
                ]),
                markov_chain.data
            );
        }

        #[test]
        fn test_remove_markov_chain() {
            let mut markov_chain = TripletMarkovChain {
                data: HashMap::from([(
                    "one two".to_string(),
                    HashMap::from([("three".to_string(), 5 as Counter)]),
                )]),
            };
            let other = TripletMarkovChain {
                data: HashMap::from([(
                    "one two".to_string(),
                    HashMap::from([("three".to_string(), 2 as Counter)]),
                )]),
            };
            markov_chain.remove_markov_chain(&other);

            let expected = TripletMarkovChain {
                data: HashMap::from([(
                    "one two".to_string(),
                    HashMap::from([("three".to_string(), 3 as Counter)]),
                )]),
            };
            assert_eq!(expected, markov_chain);
        }
    }
}
