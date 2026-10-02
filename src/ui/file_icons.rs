//! File-type icons for listings and the sidebar, using selected bundled Lucide SVGs.
use crate::{
    appearance::Tokens,
    domain::{Entry, EntryKind},
};
use gpui_kit::assets::IconName;

gpui_kit::assets::icon_assets!(
    pub ExtraIcons,
    [
        FileCode,
        FileBraces,
        FileImage,
        FileArchive,
        FileSpreadsheet,
        FileTerminal,
        FileMusic,
        FileVideoCamera,
        FileLock,
        FileCog,
        FileKey,
        FileSymlink,
        FileDiff,
        FileType,
        FolderGit,
        FolderTree,
        FolderCog,
        FolderArchive,
        FolderSymlink,
        GitBranch,
        Server,
        Cloud,
        Database
    ]
);

/// Default component icons plus the application's selected extras.
pub struct AppAssets;
impl gpui_kit::AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<gpui_kit::SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

const BLUE: (u32, u32) = (0x2f6fb8, 0x7fb0ea);
const GREEN: (u32, u32) = (0x2f8a4a, 0x86c991);
const ORANGE: (u32, u32) = (0xc0611b, 0xe8a06a);
const YELLOW: (u32, u32) = (0x9a7413, 0xe2c26e);
const PURPLE: (u32, u32) = (0x7d4fb0, 0xc2a2e6);
const RED: (u32, u32) = (0xb23a3a, 0xec8f8f);
const TEAL: (u32, u32) = (0x1f8282, 0x7ccfcf);

/// Icon and color for an entry. `expanded` selects the open-folder glyph.
pub fn entry_icon(entry: &Entry, expanded: bool, theme: &Tokens) -> (IconName, u32) {
    let name = entry.name.to_string_lossy().to_lowercase();
    let pick = |color: (u32, u32)| if theme.dark { color.1 } else { color.0 };
    match entry.kind {
        EntryKind::Directory => {
            let icon = match name.as_str() {
                _ if expanded => IconName::FolderOpen,
                ".git" => IconName::FolderGit,
                ".vscode" | ".idea" | ".github" | ".config" | ".zed" | ".cargo" => {
                    IconName::FolderCog
                }
                "node_modules" | "vendor" | "target" | "dist" | "build" => IconName::FolderArchive,
                _ => IconName::Folder,
            };
            (icon, theme.muted)
        }
        EntryKind::Symlink => (IconName::FileSymlink, theme.muted),
        EntryKind::Other => (IconName::File, theme.muted),
        EntryKind::File => file_icon(&name, pick, theme),
    }
}

fn file_icon(name: &str, pick: impl Fn((u32, u32)) -> u32, theme: &Tokens) -> (IconName, u32) {
    match name {
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitkeep" => {
            return (IconName::GitBranch, pick(ORANGE));
        }
        ".editorconfig" | ".prettierrc" | ".eslintrc" | ".npmrc" | ".nvmrc" | "makefile"
        | "dockerfile" => return (IconName::FileCog, pick(PURPLE)),
        "cargo.lock" | "composer.lock" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml"
        | "bun.lockb" | "gemfile.lock" | "poetry.lock" => {
            return (IconName::FileLock, pick(YELLOW));
        }
        _ => {}
    }
    if name.starts_with(".env") {
        return (IconName::FileKey, pick(YELLOW));
    }
    let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    match extension {
        "rs" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "php" | "py" | "rb" | "go" | "c"
        | "h" | "cpp" | "hpp" | "cc" | "swift" | "kt" | "java" | "cs" | "lua" | "zig" | "ex"
        | "exs" | "vue" | "svelte" | "dart" | "scala" => (IconName::FileCode, pick(BLUE)),
        "html" | "htm" | "xml" | "css" | "scss" | "sass" | "less" => {
            (IconName::FileCode, pick(ORANGE))
        }
        "json" | "jsonc" | "json5" | "yaml" | "yml" | "toml" | "ini" | "plist" | "conf" | "cfg" => {
            (IconName::FileBraces, pick(YELLOW))
        }
        "sh" | "bash" | "zsh" | "fish" | "command" | "ps1" => (IconName::FileTerminal, pick(GREEN)),
        "md" | "mdx" | "txt" | "rst" | "rtf" | "log" | "pdf" | "doc" | "docx" | "pages" => {
            (IconName::FileText, theme.muted)
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "icns" | "bmp" | "tiff"
        | "heic" | "avif" | "psd" => (IconName::FileImage, pick(PURPLE)),
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "dmg" | "pkg" | "iso"
        | "jar" | "deb" | "rpm" | "zst" => (IconName::FileArchive, pick(RED)),
        "csv" | "tsv" | "xls" | "xlsx" | "numbers" | "ods" => {
            (IconName::FileSpreadsheet, pick(GREEN))
        }
        "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "aiff" => {
            (IconName::FileMusic, pick(TEAL))
        }
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => (IconName::FileVideoCamera, pick(TEAL)),
        "sql" | "sqlite" | "db" | "sqlite3" => (IconName::Database, pick(TEAL)),
        "diff" | "patch" => (IconName::FileDiff, pick(GREEN)),
        "lock" => (IconName::FileLock, pick(YELLOW)),
        "ttf" | "otf" | "woff" | "woff2" => (IconName::FileType, theme.muted),
        "key" | "pem" | "crt" | "cer" | "p12" | "pub" => (IconName::FileKey, pick(YELLOW)),
        _ => (IconName::File, theme.muted),
    }
}
