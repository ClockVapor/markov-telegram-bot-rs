use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, LinkedList};

use rand::Rng;
use serde::{Deserialize, Serialize};
use substring::Substring;

use MarkovChainError::*;

pub type Counter = i64;

/*#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkovChain {
    /// HashMap of word (#1) to HashMap of following word (#2) to number of times #2 has followed #1.
    data: HashMap<String, HashMap<String, Counter>>,
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
        let first = encode_db_field_name(first);
        let second = encode_db_field_name(second);

        match self.data.get_mut(first.as_str()) {
            Some(word_map) => match word_map.get(&second) {
                Some(count) => {
                    let new_count = count + 1;
                    word_map.insert(second, new_count);
                }
                None => {
                    word_map.insert(second, 1);
                }
            },
            None => {
                let mut word_map = HashMap::new();
                word_map.insert(second, 1);
                self.data.insert(first.to_string(), word_map);
            }
        }
    }

    /// Removes a given count from a pair of words in the Markov chain.
    fn remove_word_pair(&mut self, first: &str, second: &str, amount: &Counter) {
        let first = encode_db_field_name(first);
        let second = encode_db_field_name(second);

        if let Some(word_map) = self.data.get_mut(first.as_str()) {
            if let Some(count) = word_map.get(&second) {
                let new_count = count - amount;
                if new_count > 0 {
                    word_map.insert(second, new_count);
                } else {
                    word_map.remove(&second);
                    if word_map.is_empty() {
                        self.data.remove(first.as_str());
                    }
                }
            }
        }
    }
}*/

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TripletMarkovChain {
    /// [`HashMap`] of two words (#1 and #2) to [`HashMap`] of following word (#3) to number of times
    /// #3 has followed "#1 #2".
    data: HashMap<String, HashMap<String, Counter>>,
    /// [`HashMap`] of word to [`HashSet`] of keys in `data` that end with the word. This is used to generate messages
    /// with a given seed word.
    meta: HashMap<String, HashSet<String>>,
}

impl TripletMarkovChain {
    /// Generates a [`LinkedList`] of words from the Markov chain. An optional seed word can be given to start with;
    /// otherwise, a weighted random one will be chosen based on starting words in the Markov chain.
    pub fn generate(
        &self,
        seed: Option<&String>,
        length_requirement: Option<&LengthRequirement>,
    ) -> Result<Vec<String>, MarkovChainError> {
        if self.data.is_empty() {
            return Err(Empty);
        }
        if let Some(length_requirement) = length_requirement {
            if !length_requirement.is_valid() {
                return Err(LengthRequirementInvalid);
            }
        }

        let mut starts = match seed {
            // Use the given seed word
            Some(word) => {
                let word = word.to_lowercase();
                let word_encoded = encode_db_field_name(&word);
                match self.meta.get(&word_encoded) {
                    None => {
                        return Err(NoSuchSeed);
                    }
                    Some(key_set) => {
                        // 1. Get all keys in `data` that end with `word`.
                        // 2. Build frequency map for words that follow `word`.
                        // 3. Pick random following word from the frequency map, and use DECODED version of it in pair.
                        let mut frequency_map = HashMap::<(String, String), Counter>::new();
                        for key in key_set {
                            frequency_map.insert(
                                string_to_pair(key),
                                self.data.get(key).unwrap().values().sum(),
                            );
                        }
                        frequency_map
                    }
                }
            }

            // Use the default starting point, which will select a random starting word
            None => HashMap::from([(("".to_string(), "".to_string()), 1)]),
        };

        while !starts.is_empty() {
            let start = choose_from_frequency_map(&starts).clone();
            starts.remove(&start);
            if let Ok(path) = self.generate_internal(length_requirement, start, 0) {
                let mut result = Vec::new();
                for word in path {
                    result.push(word);
                }
                return Ok(result);
            }
        }

        Err(MarkovChainError::CannotMeetLengthRequirement)
    }

    /// Internal function to recursively generate a message.
    fn generate_internal(
        &self,
        length_requirement: Option<&LengthRequirement>,
        start: (String, String),
        current_length: i32,
    ) -> Result<LinkedList<String>, MarkovChainError> {
        let at_path_end = start.0.as_str() != "" && start.1.as_str() == "";

        if let Some(length_requirement) = length_requirement {
            // If the length requirement is not satisfied, check if we need more or less words in the message
            if !length_requirement.is_satisfied_by(current_length) {
                let difference = length_requirement.difference(current_length);
                // If we need less words, or if we need more and we're at the end of a path, then this path cannot meet
                // the length requirement.
                if difference < 0 || (at_path_end && difference > 0) {
                    return Err(MarkovChainError::CannotMeetLengthRequirement);
                }
            }
        }

        if at_path_end {
            return Ok(LinkedList::new());
        } else {
            let is_real_word = start.1.as_str() != "";
            let queue = self.get_connections_in_weighted_random_order(&start);
            for pair in queue {
                let tail = self.generate_internal(
                    length_requirement,
                    pair,
                    current_length + if is_real_word { 1 } else { 0 },
                );
                if let Ok(mut tail) = tail {
                    if is_real_word {
                        tail.push_front(start.1);
                    }
                    return Ok(tail);
                }
            }
        }

        Err(MarkovChainError::CannotMeetLengthRequirement)
    }

    /// Returns a [`Vec`] of all connections for the given starting pair, in a weighted random order.
    fn get_connections_in_weighted_random_order(
        &self,
        pair: &(String, String),
    ) -> Vec<(String, String)> {
        let pair_string = pair_to_string(pair);
        match self.data.get(&pair_string) {
            Some(word_map) => {
                let mut connections = Vec::with_capacity(word_map.len());
                let second_encoded = encode_db_field_name(&pair.1);
                let mut word_map = word_map.clone();
                while !word_map.is_empty() {
                    let selected = choose_from_frequency_map(&word_map).clone();
                    word_map.remove(&selected);
                    let selected_decoded = decode_db_field_name(&selected);
                    connections.push((second_encoded.clone(), selected_decoded));
                }
                connections
            }

            None => Vec::new(),
        }
    }

    /// Adds each word pair in the given `&str` (separated by whitespace) to the Markov chain.
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
        let pair_string_encoded = pair_to_string(&pair);
        let third_encoded = encode_db_field_name(&third);

        match self.data.entry(pair_string_encoded.clone()) {
            Entry::Occupied(mut word_map_entry) => {
                match word_map_entry.get_mut().entry(third_encoded) {
                    Entry::Occupied(mut count_entry) => {
                        count_entry.insert(count_entry.get() + 1);
                    }
                    Entry::Vacant(count_entry) => {
                        count_entry.insert(1);
                    }
                }
            }
            Entry::Vacant(word_map_entry) => {
                let mut word_map = HashMap::new();
                word_map.insert(third_encoded, 1);
                word_map_entry.insert(word_map);
            }
        }

        // Keep track of the keys in `data` ending with the second word. This makes it easy to generate messages
        // with a given seed word. Also count the "cleaned" version of the second word, which is all lowercase with
        // leading and trailing non-alphanumeric characters trimmed off.
        for seed_encoded in get_associated_seeds_encoded(&pair.1) {
            if !seed_encoded.is_empty() {
                match self.meta.entry(seed_encoded) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().insert(pair_string_encoded.clone());
                    }
                    Entry::Vacant(entry) => {
                        let mut set = HashSet::new();
                        set.insert(pair_string_encoded.clone());
                        entry.insert(set);
                    }
                }
            }
        }
    }

    /// Removes a given count from a triplet of words in the Markov chain.
    fn remove_word_triplet(&mut self, pair: &(String, String), third: &str, amount: &Counter) {
        let pair_string_encoded = pair_to_string(pair);
        let third_encoded = encode_db_field_name(third);
        if let Some(word_map) = self.data.get_mut(&pair_string_encoded) {
            if let Some(count) = word_map.get(&third_encoded) {
                let new_count = count - amount;
                if new_count > 0 {
                    word_map.insert(third_encoded, new_count);
                } else {
                    word_map.remove(&third_encoded);
                    if word_map.is_empty() {
                        self.data.remove(&pair_string_encoded);

                        // The second word is no longer associated with this `data` key, since we just removed it
                        // from `data`
                        for seed_encoded in get_associated_seeds_encoded(&pair.1) {
                            let associated_keys_set = self.meta.get_mut(&seed_encoded).unwrap();
                            associated_keys_set.remove(&pair_string_encoded);
                            if associated_keys_set.is_empty() {
                                self.meta.remove(&seed_encoded);
                            }
                        }
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

    /// A seed was given for a generation operation, but the Markov chain doesn't contain the seed.
    NoSuchSeed,

    /// A length requirement was given for a generated message, but it is invalid.
    LengthRequirementInvalid,

    /// A length requirement was given for a generated message, but it couldn't be met.
    CannotMeetLengthRequirement,
    ///// Catch-all for unexpected Markov chain errors.
    //InternalError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthRequirement {
    pub value: i32,
    pub comparison_operator: ComparisonOperator,
}

impl LengthRequirement {
    /// Returns whether or not the [`LengthRequirement`] makes sense.
    pub fn is_valid(&self) -> bool {
        match self.comparison_operator {
            ComparisonOperator::LessThan => self.value > 1,
            ComparisonOperator::LessThanOrEqualTo => self.value > 0,
            ComparisonOperator::EqualTo => self.value > 0,
            ComparisonOperator::GreaterThan => self.value > 0,
            ComparisonOperator::GreaterThanOrEqualTo => self.value > 1,
        }
    }

    /// Returns the difference between `value` and `n`, the sign indicating whether more or less words are needed.
    pub fn difference(&self, n: i32) -> i32 {
        match self.comparison_operator {
            ComparisonOperator::LessThan => self.value - n - 1,
            ComparisonOperator::LessThanOrEqualTo => self.value - n,
            ComparisonOperator::EqualTo => self.value - n,
            ComparisonOperator::GreaterThan => self.value - n + 1,
            ComparisonOperator::GreaterThanOrEqualTo => self.value - n,
        }
    }

    /// Checks if the given `i32` is satisfied by this [`LengthRequirement`].
    pub fn is_satisfied_by(&self, n: i32) -> bool {
        match self.comparison_operator {
            ComparisonOperator::LessThan => n < self.value,
            ComparisonOperator::LessThanOrEqualTo => n <= self.value,
            ComparisonOperator::EqualTo => n == self.value,
            ComparisonOperator::GreaterThan => n > self.value,
            ComparisonOperator::GreaterThanOrEqualTo => n >= self.value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOperator {
    LessThan,
    LessThanOrEqualTo,
    EqualTo,
    GreaterThan,
    GreaterThanOrEqualTo,
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
    result.push(' ');
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

    match s.find(' ') {
        None => panic!("Invalid string given; contains no space: {}", s),
        Some(i) => (
            s.substring(0, i).to_string(),
            s.substring(i + 1, s.len()).to_string(),
        ),
    }
}

/// MongoDB 4 doesn't let field names start with '$'.
fn encode_db_field_name(s: &str) -> String {
    if s.starts_with('$') {
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

/// Gets a [`HashSet`] of all encoded seed words associated with the given word.
fn get_associated_seeds_encoded(decoded_word: &str) -> HashSet<String> {
    let lowercase_encoded = encode_db_field_name(&decoded_word.to_lowercase());
    let cleaned_encoded = encode_db_field_name(
        decoded_word
            .to_lowercase()
            .trim_matches(|c: char| !c.is_alphanumeric()),
    );
    HashSet::from([lowercase_encoded, cleaned_encoded])
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

            markov_chain.add_word_pair("$one", "$two");
            assert_eq!(
                HashMap::from([("\\$two".to_string(), 1 as Counter)]),
                *markov_chain.data.get("\\$one").unwrap()
            );

            markov_chain.add_word_pair("$one", "$two");
            assert_eq!(
                HashMap::from([("\\$two".to_string(), 2 as Counter)]),
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
                    HashMap::from([("\\$two".to_string(), 3 as Counter)]),
                )]),
            };

            markov_chain.remove_word_pair("$one", "$two", &(2 as Counter));
            assert_eq!(
                HashMap::from([("\\$two".to_string(), 1 as Counter)]),
                *markov_chain.data.get("\\$one").unwrap()
            );

            markov_chain.remove_word_pair("$one", "$two", &(1 as Counter));
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

        #[test]
        fn test_generate() {
            let markov_chain = MarkovChain {
                data: HashMap::from([
                    (
                        "".to_string(),
                        HashMap::from([("\\$one".to_string(), 5 as Counter)]),
                    ),
                    (
                        "\\$one".to_string(),
                        HashMap::from([("\\$two".to_string(), 5 as Counter)]),
                    ),
                    (
                        "\\$two".to_string(),
                        HashMap::from([("".to_string(), 5 as Counter)]),
                    ),
                ]),
            };
            match markov_chain.generate(None) {
                Ok(result) => {
                    assert_eq!(vec!["$one".to_string(), "$two".to_string()], result);
                }
                Err(e) => {
                    panic!("Received MarkovChainError: {:?}", e);
                }
            }
        }
    }

    mod triplet_markov_chain {
        use super::*;

        #[test]
        fn test_add_word_triplet() {
            let mut markov_chain = TripletMarkovChain::default();
            assert_eq!(HashMap::default(), markov_chain.data);
            assert_eq!(HashMap::default(), markov_chain.meta);

            markov_chain
                .add_word_triplet(("one".to_string(), "two".to_string()), "three".to_string());
            assert_eq!(
                HashMap::from([("three".to_string(), 1 as Counter)]),
                *markov_chain.data.get("one two").unwrap()
            );
            assert_eq!(
                HashSet::from(["one two".to_string()]),
                *markov_chain.meta.get("two").unwrap()
            );

            markov_chain
                .add_word_triplet(("one".to_string(), "two".to_string()), "three".to_string());
            assert_eq!(
                HashMap::from([("three".to_string(), 2 as Counter)]),
                *markov_chain.data.get("one two").unwrap()
            );
            assert_eq!(
                HashSet::from(["one two".to_string()]),
                *markov_chain.meta.get("two").unwrap()
            );
        }

        #[test]
        fn test_add_word_triplet_leading_dollar_sign() {
            let mut markov_chain = TripletMarkovChain::default();
            assert_eq!(HashMap::default(), markov_chain.data);
            assert_eq!(HashMap::default(), markov_chain.meta);

            markov_chain.add_word_triplet(
                ("$one".to_string(), "$two".to_string()),
                "$three".to_string(),
            );
            assert_eq!(
                HashMap::from([("\\$three".to_string(), 1 as Counter)]),
                *markov_chain.data.get("\\$one $two").unwrap()
            );
            assert_eq!(
                HashSet::from(["\\$one $two".to_string()]),
                *markov_chain.meta.get("\\$two").unwrap()
            );

            markov_chain.add_word_triplet(
                ("$one".to_string(), "$two".to_string()),
                "$three".to_string(),
            );
            assert_eq!(
                HashMap::from([("\\$three".to_string(), 2 as Counter)]),
                *markov_chain.data.get("\\$one $two").unwrap()
            );
            assert_eq!(
                HashSet::from(["\\$one $two".to_string()]),
                *markov_chain.meta.get("\\$two").unwrap()
            );
        }

        #[test]
        fn test_remove_word_triplet() {
            let mut markov_chain = TripletMarkovChain::default();
            for _ in 0..3 {
                markov_chain
                    .add_word_triplet(("one".to_string(), "two,".to_string()), "three".to_string());
            }
            assert_eq!(
                TripletMarkovChain {
                    data: HashMap::from([(
                        "one two,".to_string(),
                        HashMap::from([("three".to_string(), 3 as Counter)])
                    ),]),
                    meta: HashMap::from([
                        ("two,".to_string(), HashSet::from(["one two,".to_string()])),
                        ("two".to_string(), HashSet::from(["one two,".to_string()])),
                    ]),
                },
                markov_chain
            );

            markov_chain.remove_word_triplet(
                &("one".to_string(), "two,".to_string()),
                "three",
                &(2 as Counter),
            );
            assert_eq!(
                TripletMarkovChain {
                    data: HashMap::from([(
                        "one two,".to_string(),
                        HashMap::from([("three".to_string(), 1 as Counter)])
                    ),]),
                    meta: HashMap::from([
                        ("two,".to_string(), HashSet::from(["one two,".to_string()])),
                        ("two".to_string(), HashSet::from(["one two,".to_string()])),
                    ]),
                },
                markov_chain
            );
            markov_chain.remove_word_triplet(
                &("one".to_string(), "two,".to_string()),
                "three",
                &(1 as Counter),
            );
            assert_eq!(
                TripletMarkovChain {
                    data: HashMap::default(),
                    meta: HashMap::default(),
                },
                markov_chain
            );
        }

        #[test]
        fn test_remove_word_triplet_leading_dollar_sign() {
            let mut markov_chain = TripletMarkovChain::default();
            for _ in 0..3 {
                markov_chain.add_word_triplet(
                    ("$one".to_string(), "$two".to_string()),
                    "$three".to_string(),
                );
            }
            assert_eq!(
                HashMap::from([("\\$three".to_string(), 3 as Counter)]),
                *markov_chain.data.get("\\$one $two").unwrap()
            );
            assert_eq!(
                HashSet::from(["\\$one $two".to_string()]),
                *markov_chain.meta.get("\\$two").unwrap()
            );

            markov_chain.remove_word_triplet(
                &("$one".to_string(), "$two".to_string()),
                "$three",
                &(2 as Counter),
            );
            assert_eq!(
                HashMap::from([("\\$three".to_string(), 1 as Counter)]),
                *markov_chain.data.get("\\$one $two").unwrap()
            );
            assert_eq!(
                HashSet::from(["\\$one $two".to_string()]),
                *markov_chain.meta.get("\\$two").unwrap()
            );

            markov_chain.remove_word_triplet(
                &("$one".to_string(), "$two".to_string()),
                "$three",
                &(1 as Counter),
            );
            assert!(markov_chain.data.get("\\$one $two").is_none());
            assert!(markov_chain.meta.get("\\$two").is_none());
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
            assert_eq!(
                HashMap::from([
                    ("one".to_string(), HashSet::from([" one".to_string()])),
                    ("two".to_string(), HashSet::from(["one two".to_string()])),
                    (
                        "three".to_string(),
                        HashSet::from(["two three".to_string()])
                    ),
                ]),
                markov_chain.meta
            );
        }

        #[test]
        fn test_remove_markov_chain() {
            let mut markov_chain = TripletMarkovChain::default();
            for _ in 0..5 {
                markov_chain
                    .add_word_triplet(("one".to_string(), "two".to_string()), "three".to_string());
            }

            let mut other = TripletMarkovChain::default();
            for _ in 0..2 {
                other.add_word_triplet(("one".to_string(), "two".to_string()), "three".to_string());
            }

            markov_chain.remove_markov_chain(&other);

            let expected = TripletMarkovChain {
                data: HashMap::from([(
                    "one two".to_string(),
                    HashMap::from([("three".to_string(), 3 as Counter)]),
                )]),
                meta: HashMap::from([("two".to_string(), HashSet::from(["one two".to_string()]))]),
            };
            assert_eq!(expected, markov_chain);
        }

        #[test]
        fn test_generate() {
            let mut markov_chain = TripletMarkovChain::default();
            for _ in 0..5 {
                markov_chain.add_message("$one $two $three");
            }
            assert_eq!(
                TripletMarkovChain {
                    data: HashMap::from([
                        (
                            " ".to_string(),
                            HashMap::from([("\\$one".to_string(), 5 as Counter)])
                        ),
                        (
                            " $one".to_string(),
                            HashMap::from([("\\$two".to_string(), 5 as Counter)])
                        ),
                        (
                            "\\$one $two".to_string(),
                            HashMap::from([("\\$three".to_string(), 5 as Counter)])
                        ),
                        (
                            "\\$two $three".to_string(),
                            HashMap::from([("".to_string(), 5 as Counter)])
                        ),
                        (
                            "\\$three ".to_string(),
                            HashMap::from([("".to_string(), 5 as Counter)])
                        ),
                    ]),
                    meta: HashMap::from([
                        ("\\$one".to_string(), HashSet::from([" $one".to_string()])),
                        ("one".to_string(), HashSet::from([" $one".to_string()])),
                        (
                            "\\$two".to_string(),
                            HashSet::from(["\\$one $two".to_string()])
                        ),
                        (
                            "two".to_string(),
                            HashSet::from(["\\$one $two".to_string()])
                        ),
                        (
                            "\\$three".to_string(),
                            HashSet::from(["\\$two $three".to_string()])
                        ),
                        (
                            "three".to_string(),
                            HashSet::from(["\\$two $three".to_string()])
                        ),
                    ]),
                },
                markov_chain
            );

            match markov_chain.generate(None, None) {
                Ok(result) => {
                    assert_eq!(
                        vec!["$one".to_string(), "$two".to_string(), "$three".to_string()],
                        result
                    );
                }
                Err(e) => {
                    panic!("Received MarkovChainError: {:?}", e);
                }
            }

            match markov_chain.generate(Some(&"$one".to_string()), None) {
                Ok(result) => {
                    assert_eq!(
                        vec!["$one".to_string(), "$two".to_string(), "$three".to_string()],
                        result
                    );
                }
                Err(e) => {
                    panic!("Received MarkovChainError: {:?}", e);
                }
            }

            match markov_chain.generate(Some(&"$two".to_string()), None) {
                Ok(result) => {
                    assert_eq!(vec!["$two".to_string(), "$three".to_string()], result);
                }
                Err(e) => {
                    panic!("Received MarkovChainError: {:?}", e);
                }
            }

            match markov_chain.generate(Some(&"$three".to_string()), None) {
                Ok(result) => {
                    assert_eq!(vec!["$three".to_string()], result);
                }
                Err(e) => {
                    panic!("Received MarkovChainError: {:?}", e);
                }
            }
        }

        #[test]
        fn test_generate_cleaned_seed() {
            let mut markov_chain = TripletMarkovChain::default();
            markov_chain.add_message("One, two, three!");
            assert_eq!(
                TripletMarkovChain {
                    data: HashMap::from([
                        (
                            " ".to_string(),
                            HashMap::from([("One,".to_string(), 1 as Counter)])
                        ),
                        (
                            " One,".to_string(),
                            HashMap::from([("two,".to_string(), 1 as Counter)])
                        ),
                        (
                            "One, two,".to_string(),
                            HashMap::from([("three!".to_string(), 1 as Counter)])
                        ),
                        (
                            "two, three!".to_string(),
                            HashMap::from([("".to_string(), 1 as Counter)])
                        ),
                        (
                            "three! ".to_string(),
                            HashMap::from([("".to_string(), 1 as Counter)])
                        ),
                    ]),
                    meta: HashMap::from([
                        ("one,".to_string(), HashSet::from([" One,".to_string()])),
                        ("one".to_string(), HashSet::from([" One,".to_string()])),
                        ("two,".to_string(), HashSet::from(["One, two,".to_string()])),
                        ("two".to_string(), HashSet::from(["One, two,".to_string()])),
                        (
                            "three!".to_string(),
                            HashSet::from(["two, three!".to_string()])
                        ),
                        (
                            "three".to_string(),
                            HashSet::from(["two, three!".to_string()])
                        ),
                    ]),
                },
                markov_chain
            );

            for seed in vec!["One,", "one,", "One", "one"] {
                match markov_chain.generate(Some(&seed.to_string()), None) {
                    Ok(result) => {
                        assert_eq!(
                            vec!["One,".to_string(), "two,".to_string(), "three!".to_string()],
                            result
                        );
                    }
                    Err(e) => {
                        panic!("Received MarkovChainError: {:?}", e);
                    }
                }
            }
        }
    }
}
