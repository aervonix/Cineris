//! Which language a file is in: by its name, and for a script with no
//! extension by its first line.
//!
//! The fifteen are chosen from what users of a coding agent meet most:
//! the languages with the most activity on public code hosts (Python,
//! JavaScript, TypeScript, Java, C#, C, C++, Go, Rust, PHP, Ruby, Kotlin,
//! Swift, shell), and Markdown, because nearly every repository documents
//! itself in it and its headings are its outline. Every other text file is
//! outlined by indentation.

/// A language with an extractor of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    /// Rust.
    Rust,
    /// Python.
    Python,
    /// JavaScript, with JSX.
    JavaScript,
    /// TypeScript, with TSX.
    TypeScript,
    /// Java.
    Java,
    /// C#.
    CSharp,
    /// C.
    C,
    /// C++.
    Cpp,
    /// Go.
    Go,
    /// PHP.
    Php,
    /// Ruby.
    Ruby,
    /// Kotlin.
    Kotlin,
    /// Swift.
    Swift,
    /// The POSIX shell and bash.
    Shell,
    /// Markdown.
    Markdown,
}

impl Language {
    /// Every language, in the order the cache numbers them.
    pub const ALL: [Self; 15] = [
        Self::Rust,
        Self::Python,
        Self::JavaScript,
        Self::TypeScript,
        Self::Java,
        Self::CSharp,
        Self::C,
        Self::Cpp,
        Self::Go,
        Self::Php,
        Self::Ruby,
        Self::Kotlin,
        Self::Swift,
        Self::Shell,
        Self::Markdown,
    ];

    /// The short name the cache and the map use.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Go => "go",
            Self::Php => "php",
            Self::Ruby => "ruby",
            Self::Kotlin => "kotlin",
            Self::Swift => "swift",
            Self::Shell => "shell",
            Self::Markdown => "markdown",
        }
    }

    /// The language a short name names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.name() == name)
    }

    /// The language of a file by its name alone. Case is ignored in the
    /// extension, so `MAIN.C` is C, as Windows would have it.
    #[must_use]
    pub fn of_name(file_name: &str) -> Option<Self> {
        let extension = file_name
            .rsplit_once('.')
            .filter(|(stem, _)| !stem.is_empty())
            .map(|(_, extension)| extension.to_ascii_lowercase())?;
        Some(match extension.as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" | "pyw" => Self::Python,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "tsx" | "mts" | "cts" => Self::TypeScript,
            "java" => Self::Java,
            "cs" => Self::CSharp,
            "c" => Self::C,
            // A `.h` may be C or C++: the C++ extractor reads both.
            "h" | "hh" | "hpp" | "hxx" | "h++" | "cc" | "cpp" | "cxx" | "c++" | "ipp" | "tpp" => {
                Self::Cpp
            }
            "go" => Self::Go,
            "php" | "phtml" => Self::Php,
            "rb" | "rake" | "gemspec" => Self::Ruby,
            "kt" | "kts" => Self::Kotlin,
            "swift" => Self::Swift,
            "sh" | "bash" | "zsh" | "ksh" => Self::Shell,
            "md" | "markdown" | "mdx" => Self::Markdown,
            _ => return None,
        })
    }

    /// The language of a file by its name, and when the name says nothing,
    /// by a `#!` line at its start.
    #[must_use]
    pub fn detect(file_name: &str, text: &str) -> Option<Self> {
        if let Some(language) = Self::of_name(file_name) {
            return Some(language);
        }
        if matches!(file_name, "Rakefile" | "Gemfile") {
            return Some(Self::Ruby);
        }
        let first = text.lines().next()?.strip_prefix("#!")?;
        // `#!/usr/bin/env -S python3 -u` names its program after `env`.
        let mut words = first.split_whitespace();
        let mut program = words.next()?.rsplit('/').next()?;
        while program == "env" || program.starts_with('-') {
            program = words.next()?;
        }
        let program = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        Some(match program {
            "sh" | "bash" | "zsh" | "ksh" | "dash" => Self::Shell,
            "python" => Self::Python,
            "node" | "deno" | "bun" => Self::JavaScript,
            "ruby" => Self::Ruby,
            "php" => Self::Php,
            _ => return None,
        })
    }

    /// Whether a `.h` file is the kind of file this language's extractor
    /// reads declarations in: a header, where a prototype is the interface.
    #[must_use]
    pub fn is_header(file_name: &str) -> bool {
        file_name.rsplit_once('.').is_some_and(|(_, extension)| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "h" | "hh" | "hpp" | "hxx" | "h++"
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_reads_back() {
        for language in Language::ALL {
            assert_eq!(Language::from_name(language.name()), Some(language));
        }
    }

    #[test]
    fn languages_are_known_by_their_extensions() {
        assert_eq!(Language::of_name("main.rs"), Some(Language::Rust));
        assert_eq!(Language::of_name("MAIN.C"), Some(Language::C));
        assert_eq!(Language::of_name("view.tsx"), Some(Language::TypeScript));
        assert_eq!(Language::of_name("README.md"), Some(Language::Markdown));
        assert_eq!(Language::of_name("a.h"), Some(Language::Cpp));
        // A dotfile has no extension, and an unknown one is no language.
        assert_eq!(Language::of_name(".rs"), None);
        assert_eq!(Language::of_name("notes.txt"), None);
        assert_eq!(Language::of_name("Makefile"), None);
    }

    #[test]
    fn a_script_without_an_extension_is_known_by_its_first_line() {
        assert_eq!(
            Language::detect("deploy", "#!/bin/bash\necho hi\n"),
            Some(Language::Shell)
        );
        assert_eq!(
            Language::detect("tool", "#!/usr/bin/env python3\n"),
            Some(Language::Python)
        );
        assert_eq!(
            Language::detect("tool", "#!/usr/bin/env -S node --no-warnings\n"),
            Some(Language::JavaScript)
        );
        assert_eq!(Language::detect("tool", "#!/usr/bin/perl\n"), None);
        assert_eq!(Language::detect("tool", "no line"), None);
        assert_eq!(Language::detect("Rakefile", ""), Some(Language::Ruby));
    }
}
