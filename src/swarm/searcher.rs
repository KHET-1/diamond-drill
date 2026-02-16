//! Swarm Searcher - Hybrid Keyword + Vector Search
//!
//! Combines multiple search strategies:
//! - Exact keyword matching (regex)
//! - Fuzzy text matching
//! - Vector similarity (cosine)
//! - Hybrid scoring with configurable weights

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use parking_lot::RwLock;
use rayon::prelude::*;
use regex::Regex;
use tracing::{debug, info};

use super::embedder::{cosine_similarity, Embedder};

// ============================================================================
// Search Configuration
// ============================================================================

/// Configuration for hybrid search
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Weight for keyword matches (0.0 - 1.0)
    pub keyword_weight: f32,
    /// Weight for vector similarity (0.0 - 1.0)
    pub vector_weight: f32,
    /// Minimum score to include in results
    pub min_score: f32,
    /// Maximum results to return
    pub max_results: usize,
    /// Enable fuzzy matching
    pub fuzzy: bool,
    /// Fuzzy threshold (0.0 - 1.0)
    pub fuzzy_threshold: f32,
    /// Boost recent documents
    pub recency_boost: bool,
    /// Case insensitive keyword search
    pub case_insensitive: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            keyword_weight: 0.3,
            vector_weight: 0.7,
            min_score: 0.1,
            max_results: 100,
            fuzzy: true,
            fuzzy_threshold: 0.7,
            recency_boost: false,
            case_insensitive: true,
        }
    }
}

// ============================================================================
// Search Index
// ============================================================================

/// Document in the search index
#[derive(Debug, Clone)]
pub struct IndexedDocument {
    /// Document ID
    pub id: String,
    /// Source file path
    pub source: PathBuf,
    /// Chunk ID within source
    pub chunk_id: usize,
    /// Document text content
    pub content: String,
    /// Pre-computed embedding vector
    pub embedding: Vec<f32>,
    /// Metadata for filtering
    pub metadata: HashMap<String, String>,
    /// Index timestamp
    pub indexed_at: chrono::DateTime<chrono::Utc>,
}

/// Search result
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Document ID
    pub id: String,
    /// Source path
    pub source: PathBuf,
    /// Chunk ID
    pub chunk_id: usize,
    /// Content snippet
    pub snippet: String,
    /// Overall score
    pub score: f32,
    /// Keyword match score
    pub keyword_score: f32,
    /// Vector similarity score
    pub vector_score: f32,
    /// Matched terms (for highlighting)
    pub matched_terms: Vec<String>,
}

/// In-memory search index
pub struct SearchIndex {
    /// All indexed documents
    documents: Arc<RwLock<Vec<IndexedDocument>>>,
    /// Embedder for query vectorization
    embedder: Arc<dyn Embedder>,
    /// Inverted index for keyword search
    inverted_index: Arc<RwLock<HashMap<String, Vec<usize>>>>,
    /// Configuration
    config: SearchConfig,
}

impl SearchIndex {
    /// Create a new search index
    pub fn new(embedder: Arc<dyn Embedder>, config: SearchConfig) -> Self {
        Self {
            documents: Arc::new(RwLock::new(Vec::new())),
            embedder,
            inverted_index: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Add a document to the index
    pub fn add(&self, doc: IndexedDocument) {
        let doc_idx = {
            let mut docs = self.documents.write();
            let idx = docs.len();
            docs.push(doc.clone());
            idx
        };

        // Update inverted index
        self.update_inverted_index(&doc.content, doc_idx);
    }

    /// Add multiple documents
    pub fn add_batch(&self, docs: Vec<IndexedDocument>) {
        let start_idx = {
            let mut stored = self.documents.write();
            let start = stored.len();
            stored.extend(docs.clone());
            start
        };

        // Update inverted index in parallel
        let updates: Vec<(usize, Vec<String>)> = docs
            .par_iter()
            .enumerate()
            .map(|(i, doc)| (start_idx + i, tokenize(&doc.content)))
            .collect();

        let mut index = self.inverted_index.write();
        for (doc_idx, terms) in updates {
            for term in terms {
                index.entry(term).or_default().push(doc_idx);
            }
        }
    }

    /// Search with hybrid ranking
    pub fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        info!("Searching: {:?}", query);

        // Get query embedding
        let query_embedding = self.embedder.embed(query)?;

        // Get keyword matches
        let keyword_results = self.keyword_search(query);

        // Get vector matches
        let vector_results = self.vector_search(&query_embedding);

        // Merge and rank results
        let merged = self.merge_results(query, keyword_results, vector_results);

        debug!("Found {} results", merged.len());
        Ok(merged)
    }

    /// Keyword-only search
    fn keyword_search(&self, query: &str) -> HashMap<usize, f32> {
        let mut scores: HashMap<usize, f32> = HashMap::new();
        let query_terms = tokenize(query);
        let index = self.inverted_index.read();
        let total_docs = self.documents.read().len() as f32;

        for term in &query_terms {
            if let Some(doc_indices) = index.get(term) {
                // TF-IDF scoring
                let idf = (total_docs / doc_indices.len() as f32).ln() + 1.0;

                for &doc_idx in doc_indices {
                    let tf = 1.0; // Simplified - could count occurrences
                    *scores.entry(doc_idx).or_insert(0.0) += tf * idf;
                }
            }

            // Fuzzy matching if enabled
            if self.config.fuzzy {
                for (indexed_term, doc_indices) in index.iter() {
                    let similarity = fuzzy_similarity(term, indexed_term);
                    if similarity >= self.config.fuzzy_threshold && similarity < 1.0 {
                        let idf = (total_docs / doc_indices.len() as f32).ln() + 1.0;
                        for &doc_idx in doc_indices {
                            *scores.entry(doc_idx).or_insert(0.0) += similarity * idf * 0.5;
                        }
                    }
                }
            }
        }

        // Normalize scores
        if let Some(max_score) = scores.values().cloned().reduce(f32::max) {
            if max_score > 0.0 {
                for score in scores.values_mut() {
                    *score /= max_score;
                }
            }
        }

        scores
    }

    /// Vector similarity search
    fn vector_search(&self, query_embedding: &[f32]) -> HashMap<usize, f32> {
        let docs = self.documents.read();

        docs.par_iter()
            .enumerate()
            .map(|(idx, doc)| {
                let similarity = cosine_similarity(query_embedding, &doc.embedding);
                (idx, similarity)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect()
    }

    /// Merge keyword and vector results with hybrid scoring
    fn merge_results(
        &self,
        query: &str,
        keyword_scores: HashMap<usize, f32>,
        vector_scores: HashMap<usize, f32>,
    ) -> Vec<SearchResult> {
        let docs = self.documents.read();
        let query_terms: Vec<String> = tokenize(query);

        // Combine all document indices
        let mut all_indices: Vec<usize> = keyword_scores
            .keys()
            .chain(vector_scores.keys())
            .cloned()
            .collect();
        all_indices.sort_unstable();
        all_indices.dedup();

        // Calculate hybrid scores
        let mut results: Vec<SearchResult> = all_indices
            .par_iter()
            .filter_map(|&idx| {
                let doc = docs.get(idx)?;

                let kw_score = keyword_scores.get(&idx).cloned().unwrap_or(0.0);
                let vec_score = vector_scores.get(&idx).cloned().unwrap_or(0.0);

                let hybrid_score =
                    self.config.keyword_weight * kw_score + self.config.vector_weight * vec_score;

                if hybrid_score < self.config.min_score {
                    return None;
                }

                // Find matched terms for highlighting
                let matched: Vec<String> = query_terms
                    .iter()
                    .filter(|t| doc.content.to_lowercase().contains(&t.to_lowercase()))
                    .cloned()
                    .collect();

                // Create snippet around first match
                let snippet = create_snippet(&doc.content, &matched, 150);

                Some(SearchResult {
                    id: doc.id.clone(),
                    source: doc.source.clone(),
                    chunk_id: doc.chunk_id,
                    snippet,
                    score: hybrid_score,
                    keyword_score: kw_score,
                    vector_score: vec_score,
                    matched_terms: matched,
                })
            })
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Apply recency boost if enabled
        if self.config.recency_boost {
            // Boost more recent documents slightly
            let now = chrono::Utc::now();
            for result in &mut results {
                if let Some(doc) = docs.iter().find(|d| d.id == result.id) {
                    let age_hours = (now - doc.indexed_at).num_hours() as f32;
                    let recency_factor = 1.0 / (1.0 + age_hours / 24.0);
                    result.score *= 1.0 + 0.1 * recency_factor;
                }
            }
            results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // Limit results
        results.truncate(self.config.max_results);
        results
    }

    /// Update the inverted index for a document
    fn update_inverted_index(&self, content: &str, doc_idx: usize) {
        let terms = tokenize(content);
        let mut index = self.inverted_index.write();

        for term in terms {
            index.entry(term).or_default().push(doc_idx);
        }
    }

    /// Get index statistics
    pub fn stats(&self) -> IndexStats {
        let docs = self.documents.read();
        let index = self.inverted_index.read();

        IndexStats {
            document_count: docs.len(),
            term_count: index.len(),
            total_embeddings: docs.iter().filter(|d| !d.embedding.is_empty()).count(),
        }
    }

    /// Clear the index
    pub fn clear(&self) {
        self.documents.write().clear();
        self.inverted_index.write().clear();
    }
}

#[derive(Debug)]
pub struct IndexStats {
    pub document_count: usize,
    pub term_count: usize,
    pub total_embeddings: usize,
}

// ============================================================================
// Regex Search
// ============================================================================

/// Search with regex pattern
pub fn regex_search(pattern: &str, documents: &[IndexedDocument]) -> Result<Vec<SearchResult>> {
    let regex = Regex::new(pattern)?;

    let results: Vec<SearchResult> = documents
        .par_iter()
        .filter_map(|doc| {
            let matches: Vec<_> = regex.find_iter(&doc.content).collect();
            if matches.is_empty() {
                return None;
            }

            let matched_terms: Vec<String> =
                matches.iter().map(|m| m.as_str().to_string()).collect();

            // Score based on match count
            let score = (matches.len() as f32).ln() / 10.0 + 0.5;

            Some(SearchResult {
                id: doc.id.clone(),
                source: doc.source.clone(),
                chunk_id: doc.chunk_id,
                snippet: create_snippet(&doc.content, &matched_terms, 150),
                score: score.min(1.0),
                keyword_score: score.min(1.0),
                vector_score: 0.0,
                matched_terms,
            })
        })
        .collect();

    Ok(results)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Tokenize text into searchable terms
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(|s| s.to_string())
        .collect()
}

/// Calculate fuzzy similarity between two strings (Jaro-Winkler simplified)
fn fuzzy_similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    let match_distance = (a_chars.len().max(b_chars.len()) / 2).saturating_sub(1);

    let mut a_matches = vec![false; a_chars.len()];
    let mut b_matches = vec![false; b_chars.len()];
    let mut matches = 0;
    let mut transpositions = 0;

    for (i, &a_char) in a_chars.iter().enumerate() {
        let start = i.saturating_sub(match_distance);
        let end = (i + match_distance + 1).min(b_chars.len());

        for j in start..end {
            if b_matches[j] || a_char != b_chars[j] {
                continue;
            }
            a_matches[i] = true;
            b_matches[j] = true;
            matches += 1;
            break;
        }
    }

    if matches == 0 {
        return 0.0;
    }

    let mut j = 0;
    for (i, _) in a_chars.iter().enumerate() {
        if !a_matches[i] {
            continue;
        }
        while !b_matches[j] {
            j += 1;
        }
        if a_chars[i] != b_chars[j] {
            transpositions += 1;
        }
        j += 1;
    }

    let m = matches as f32;
    let t = transpositions as f32 / 2.0;

    (m / a_chars.len() as f32 + m / b_chars.len() as f32 + (m - t) / m) / 3.0
}

/// Create a snippet around matched terms
fn create_snippet(content: &str, matched_terms: &[String], max_len: usize) -> String {
    if matched_terms.is_empty() {
        // Return start of content
        return content.chars().take(max_len).collect();
    }

    let content_lower = content.to_lowercase();

    // Find first match position
    let first_match_pos = matched_terms
        .iter()
        .filter_map(|term| content_lower.find(&term.to_lowercase()))
        .min()
        .unwrap_or(0);

    // Calculate snippet start (try to center on match)
    let half_len = max_len / 2;
    let start = first_match_pos.saturating_sub(half_len);

    // Get snippet
    let snippet: String = content.chars().skip(start).take(max_len).collect();

    // Add ellipsis if truncated
    let mut result = String::new();
    if start > 0 {
        result.push_str("...");
    }
    result.push_str(&snippet);
    if start + max_len < content.len() {
        result.push_str("...");
    }

    result
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::embedder::Blake3Embedder;

    fn create_test_index() -> SearchIndex {
        let embedder = Arc::new(Blake3Embedder::new(384));
        let config = SearchConfig::default();
        SearchIndex::new(embedder, config)
    }

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Hello, World! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        assert!(!tokens.contains(&"a".to_string())); // Too short
    }

    #[test]
    fn test_fuzzy_similarity() {
        assert!((fuzzy_similarity("hello", "hello") - 1.0).abs() < 0.001);
        assert!(fuzzy_similarity("hello", "hallo") > 0.8);
        assert!(fuzzy_similarity("hello", "world") < 0.5);
    }

    #[test]
    fn test_search_index() {
        let index = create_test_index();
        let embedder = Arc::new(Blake3Embedder::new(384));

        // Add documents
        index.add(IndexedDocument {
            id: "doc1".to_string(),
            source: PathBuf::from("/test/doc1.txt"),
            chunk_id: 0,
            content: "The quick brown fox jumps over the lazy dog".to_string(),
            embedding: embedder.embed("The quick brown fox").unwrap(),
            metadata: HashMap::new(),
            indexed_at: chrono::Utc::now(),
        });

        index.add(IndexedDocument {
            id: "doc2".to_string(),
            source: PathBuf::from("/test/doc2.txt"),
            chunk_id: 0,
            content: "A slow yellow turtle crawls under the active cat".to_string(),
            embedding: embedder.embed("A slow yellow turtle").unwrap(),
            metadata: HashMap::new(),
            indexed_at: chrono::Utc::now(),
        });

        // Search
        let results = index.search("quick fox").unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "doc1");
    }

    #[test]
    fn test_create_snippet() {
        let content = "This is a long document with some important keywords embedded in it.";
        let matched = vec!["important".to_string()];

        let snippet = create_snippet(content, &matched, 30);
        assert!(snippet.contains("important"));
    }

    #[test]
    fn test_regex_search() {
        let embedder = Arc::new(Blake3Embedder::new(384));
        let docs = vec![IndexedDocument {
            id: "doc1".to_string(),
            source: PathBuf::from("/test/doc1.txt"),
            chunk_id: 0,
            content: "Error: file not found at line 42".to_string(),
            embedding: embedder.embed("error").unwrap(),
            metadata: HashMap::new(),
            indexed_at: chrono::Utc::now(),
        }];

        let results = regex_search(r"line \d+", &docs).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].matched_terms.contains(&"line 42".to_string()));
    }
}
