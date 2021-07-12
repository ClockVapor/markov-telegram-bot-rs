use std::collections::HashMap;

use rand::prelude::IteratorRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct MarkovChain {
    /// ID of user who owns this markov chain.
    pub user_id: String,

    /// HashMap of word (#1) to HashMap of following word (#2) to number of times #2 followed #1.
    pub markov_chain: HashMap<String, HashMap<String, i32>>,
}

impl MarkovChain {
    pub fn generate(&self, seed: Option<String>) -> Result<Vec<String>, MarkovChainError> {
        if self.markov_chain.is_empty() {
            return Err(MarkovChainError::Empty);
        }
        let mut word = match seed {
            // Use the given seed word
            Some(word) => {
                if !self.markov_chain.contains_key(&word) {
                    return Err(MarkovChainError::NoSuchSeed);
                }
                word
            }
            // Pick a random starting seed word
            None => {
                match self.markov_chain.get("") {
                    None => return Err(MarkovChainError::Empty),
                    Some(word_map) => word_map.keys().choose(&mut rand::thread_rng()).unwrap().clone(),
                }
            }
        };

        let mut result: Vec<String> = vec![];
        while word != "" {
            result.push(word.clone());
            match self.markov_chain.get(&word) {
                None => { // Should never happen based on how we build the markov chains
                    println!("Expected word {} to be in the markov chain but it wasn't", word);
                    return Err(MarkovChainError::InternalError);
                }
                Some(word_map) => {
                    let mut cumulative_distribution: Vec<(i32, &String)> = vec![];
                    let mut n = 0;
                    for (following_word, count) in word_map {
                        n += count;
                        cumulative_distribution.push((n, following_word));
                    }
                    let random = rand::thread_rng().gen_range(0..n);
                    let mut next_word: Option<String> = None;
                    for (cumulative_value, following_word) in cumulative_distribution {
                        if random < cumulative_value {
                            next_word = Some(following_word.clone());
                            break;
                        }
                    }
                    if next_word == None { // Should never happen
                        println!("Failed to pick next word in cumulative distribution");
                        return Err(MarkovChainError::InternalError);
                    }
                    word = next_word.unwrap();
                }
            }
        }

        Ok(result)
    }

    /// Adds each word pair in the given String (separated by whitespace) to the markov chain.
    pub fn add_message(&mut self, text: &String) {
        let mut words = text.split_whitespace().peekable();
        if let Some(_) = words.peek() {
            let mut last_word = "";
            for word in words {
                self.add_word_pair(&last_word.to_owned(), &word.to_owned());
                last_word = word;
            }
            self.add_word_pair(&last_word.to_owned(), &"".to_owned());
        }
    }

    /// Adds a pair of words to the markov chain.
    fn add_word_pair(&mut self, first: &String, second: &String) {
        match self.markov_chain.get_mut(first) {
            Some(word_map) => match word_map.get(second) {
                Some(count) => {
                    let new_count = count + 1;
                    word_map.insert(second.clone(), new_count);
                }
                None => {
                    word_map.insert(second.clone(), 1);
                }
            },
            None => {
                let mut word_map = HashMap::new();
                word_map.insert(second.clone(), 1);
                self.markov_chain.insert(first.clone(), word_map);
            }
        }
    }
}

#[derive(Debug)]
pub enum MarkovChainError {
    /// An operation was attempted on an empty markov chain.
    Empty,

    /// A seed was given for a markov chain operation, but the markov chain doesn't contain
    /// the seed.
    NoSuchSeed,

    /// Catch-all for unexpected markov chain errors.
    InternalError,
}
