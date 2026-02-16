//! File tree for TUI display
//!
//! Converts flat file paths into a navigable tree structure.

use crate::core::FileType;

/// A node in the file tree
#[derive(Debug, Clone)]
pub struct TreeNode {
    /// Display name (file/folder name)
    pub name: String,
    /// Full path
    pub path: String,
    /// Whether this is a directory
    pub is_dir: bool,
    /// File type (for icons/colors)
    pub file_type: FileType,
    /// Depth in the tree
    pub depth: usize,
}

/// Flat file tree with cursor-based navigation
#[derive(Debug)]
pub struct FileTree {
    /// All nodes (flattened)
    nodes: Vec<TreeNode>,
    /// Filtered nodes (indices into `nodes`)
    visible: Vec<usize>,
    /// Current selection index (into `visible`)
    selected: usize,
}

impl Default for FileTree {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTree {
    /// Create empty file tree
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            visible: Vec::new(),
            selected: 0,
        }
    }

    /// Build file tree from a list of paths
    pub fn from_paths(paths: &[String]) -> Self {
        let mut nodes = Vec::with_capacity(paths.len());

        for path in paths {
            let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();

            let ext = name.rsplit('.').next().unwrap_or("").to_string();

            let depth = path.matches(['/', '\\']).count();

            nodes.push(TreeNode {
                name,
                path: path.clone(),
                is_dir: false,
                file_type: FileType::from_extension(&ext),
                depth,
            });
        }

        // Sort by path for consistent display
        nodes.sort_by(|a, b| a.path.cmp(&b.path));

        let visible: Vec<usize> = (0..nodes.len()).collect();

        Self {
            nodes,
            visible,
            selected: 0,
        }
    }

    /// Get visible node count
    pub fn visible_count(&self) -> usize {
        self.visible.len()
    }

    /// Get current selection index
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Get the path of the currently selected node
    pub fn selected_path(&self) -> Option<String> {
        self.visible
            .get(self.selected)
            .and_then(|&idx| self.nodes.get(idx))
            .map(|n| n.path.clone())
    }

    /// Get visible nodes for rendering
    pub fn visible_nodes(&self) -> Vec<&TreeNode> {
        self.visible
            .iter()
            .filter_map(|&idx| self.nodes.get(idx))
            .collect()
    }

    /// Get a window of visible nodes around the selection for scrolling
    pub fn visible_window(&self, height: usize) -> (Vec<&TreeNode>, usize) {
        let total = self.visible.len();
        if total == 0 {
            return (Vec::new(), 0);
        }

        let half = height / 2;
        let start = if self.selected > half {
            (self.selected - half).min(total.saturating_sub(height))
        } else {
            0
        };

        let end = (start + height).min(total);

        let nodes: Vec<&TreeNode> = self.visible[start..end]
            .iter()
            .filter_map(|&idx| self.nodes.get(idx))
            .collect();

        let relative_selected = self.selected - start;
        (nodes, relative_selected)
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.visible.is_empty() && self.selected < self.visible.len() - 1 {
            self.selected += 1;
        }
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Jump to first
    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    /// Jump to last
    pub fn select_last(&mut self) {
        if !self.visible.is_empty() {
            self.selected = self.visible.len() - 1;
        }
    }

    /// Apply a filter pattern (fuzzy match on filename)
    pub fn apply_filter(&mut self, pattern: &str) {
        if pattern.is_empty() {
            self.clear_filter();
            return;
        }

        let pattern_lower = pattern.to_lowercase();
        self.visible = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.name.to_lowercase().contains(&pattern_lower))
            .map(|(idx, _)| idx)
            .collect();

        self.selected = 0;
    }

    /// Clear filter and show all nodes
    pub fn clear_filter(&mut self) {
        self.visible = (0..self.nodes.len()).collect();
        self.selected = 0;
    }

    /// Collapse (no-op for flat list - reserved for future tree expansion)
    pub fn collapse(&mut self) {
        // Reserved for hierarchical tree view
    }

    /// Expand (no-op for flat list - reserved for future tree expansion)
    pub fn expand(&mut self) {
        // Reserved for hierarchical tree view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_tree_from_paths() {
        let paths = vec![
            "/docs/readme.md".to_string(),
            "/docs/guide.pdf".to_string(),
            "/photos/vacation.jpg".to_string(),
        ];

        let tree = FileTree::from_paths(&paths);
        assert_eq!(tree.visible_count(), 3);
        assert_eq!(tree.selected_index(), 0);
    }

    #[test]
    fn test_file_tree_navigation() {
        let paths = vec![
            "a.txt".to_string(),
            "b.txt".to_string(),
            "c.txt".to_string(),
        ];

        let mut tree = FileTree::from_paths(&paths);

        tree.select_next();
        assert_eq!(tree.selected_index(), 1);

        tree.select_next();
        assert_eq!(tree.selected_index(), 2);

        // Can't go past end
        tree.select_next();
        assert_eq!(tree.selected_index(), 2);

        tree.select_prev();
        assert_eq!(tree.selected_index(), 1);

        tree.select_first();
        assert_eq!(tree.selected_index(), 0);

        tree.select_last();
        assert_eq!(tree.selected_index(), 2);
    }

    #[test]
    fn test_file_tree_filter() {
        let paths = vec![
            "photo.jpg".to_string(),
            "readme.md".to_string(),
            "photo_2.jpg".to_string(),
            "video.mp4".to_string(),
        ];

        let mut tree = FileTree::from_paths(&paths);
        assert_eq!(tree.visible_count(), 4);

        tree.apply_filter("photo");
        assert_eq!(tree.visible_count(), 2);

        tree.clear_filter();
        assert_eq!(tree.visible_count(), 4);
    }

    #[test]
    fn test_file_tree_selected_path() {
        let paths = vec!["a.txt".to_string(), "b.txt".to_string()];

        let mut tree = FileTree::from_paths(&paths);
        assert_eq!(tree.selected_path(), Some("a.txt".to_string()));

        tree.select_next();
        assert_eq!(tree.selected_path(), Some("b.txt".to_string()));
    }
}
