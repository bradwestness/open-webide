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
        "numeric.ts",
        "function main(): number {\n    // 文😀\n    return 1;\n}\n",
        FoldRange {
            start_line: 0,
            end_line: 3,
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

/// Literal text and executable interpolation markers for shared native/browser contracts.
pub const LITERAL_CASES: &[(&str, &str, &str, Option<&str>)] = &[
    (
        "literal.php",
        "<?php $s = <<<TEXT\n文😀 literal {\n{$user->name}\nTEXT;\n",
        "literal",
        Some("user"),
    ),
    (
        "literal.php",
        "<?php $s = <<<'TEXT'\n文😀 literal {$user->name}\nTEXT;\n",
        "user",
        None,
    ),
    (
        "literal.php",
        "<?php $s = \"文😀 literal {$items[call()]} tail\";",
        "literal",
        Some("call"),
    ),
    (
        "literal.sh",
        "cat <<TEXT\n文😀 literal { $(printf 'inner') }\nTEXT\n",
        "literal",
        Some("printf"),
    ),
    (
        "literal.sh",
        "cat <<'TEXT'\n文😀 literal { $(printf 'inner') }\nTEXT\n",
        "printf",
        None,
    ),
    (
        "literal.sh",
        "echo $\"文😀 literal $(printf 'inner') ${value} tail\"",
        "literal",
        Some("printf"),
    ),
    (
        "literal.sh",
        "echo \"文😀 literal $((1 + value)) tail\"",
        "literal",
        Some("value"),
    ),
    (
        "literal.py",
        "s = f\"文😀 literal {call('inner')} tail\"",
        "literal",
        Some("call"),
    ),
    (
        "literal.cs",
        "class C { string s = $\"文😀 literal {Call(\"inner\")} tail\"; }",
        "literal",
        Some("Call"),
    ),
];

/// Configuration and documentation grammars share the same worker/adapter contracts.
pub const CONFIG_CASES: &[(&str, &str)] = &[
    (
        "package.json",
        "{\n  \"name\": \"文😀\",\n  \"enabled\": true\n}\n",
    ),
    (
        "tsconfig.jsonc",
        "{\n  // compiler settings\n  \"compilerOptions\": { \"strict\": true }\n}\n",
    ),
    (
        "compose.yml",
        "services:\n  app:\n    image: \"文😀\"\n    enabled: true\n",
    ),
    (
        "config.yaml",
        "message: |\n  text 文😀\n  # prose, not a comment\nenabled: true\n",
    ),
    ("Cargo.toml", "[package]\nname = \"文😀\"\nenabled = true\n"),
    (
        "pyproject.toml",
        "[project]\nname = \"文😀\"\ndependencies = [\n  \"example\",\n]\n",
    ),
    (
        ".editorconfig",
        "root = true\n[*]\nindent_style = space\n# 文😀\nindent_size = 4\n",
    ),
    ("settings.ini", "[app]\nname = 文😀\nenabled = true\n"),
    (
        "NuGet.Config",
        "<configuration>\n  <packageSources>\n    <add key=\"文😀\" value=\"https://example.test\" />\n  </packageSources>\n</configuration>\n",
    ),
    (
        "App.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n</Project>\n",
    ),
    (
        "README.md",
        "# Project 文😀\n\nPlain words with **emphasis** and `inline code`.\n\n```rust\nfn main() { let value = 42; }\n```\n\n[docs](https://example.test)\n",
    ),
];
