//! Shared source fixtures for native and both-mode browser grammar contracts.
use super::FoldRange;

pub const LANGUAGE_CASES: &[(&str, &str, FoldRange)] = &[
    (
        "main.rs",
        "fn main() {\n    let text = \"文😀\";\n}\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "main.ts",
        "function main(): string {\n    return \"文😀\";\n}\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "view.tsx",
        "const view = <section>\n    <span>文😀</span>\n</section>;\n",
        FoldRange {
            start_line: 0,
            end_line: 1,
        },
    ),
    (
        "main.js",
        "function main() {\n    return \"文😀\";\n}\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "view.jsx",
        "const view = <section>\n    <span>文😀</span>\n</section>;\n",
        FoldRange {
            start_line: 0,
            end_line: 1,
        },
    ),
    (
        "main.py",
        "def main():\n    text = \"文😀\"\n    return text\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "Main.java",
        "class Main {\n    String text = \"文😀\";\n}\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "Main.cs",
        "class Main {\n    string text = \"文😀\";\n}\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "main.cpp",
        "int main() {\n    auto text = \"文😀\";\n}\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "main.c",
        "int main() {\n    char *text = \"文😀\";\n}\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "main.php",
        "<?php\nfunction main() {\n    echo \"文😀\";\n}\n",
        FoldRange {
            start_line: 1,
            end_line: 3,
        },
    ),
    (
        "main.sh",
        "if true; then\n    printf '%s' '文😀'\nfi\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "main.go",
        "package main\nfunc main() {\n    println(\"文😀\")\n}\n",
        FoldRange {
            start_line: 1,
            end_line: 3,
        },
    ),
    (
        "index.html",
        "<section>\n    <p>文😀</p>\n</section>\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
    (
        "style.css",
        ".main {\n    content: \"文😀\";\n}\n",
        FoldRange {
            start_line: 0,
            end_line: 2,
        },
    ),
];
