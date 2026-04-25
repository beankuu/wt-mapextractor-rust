use clap::Parser;

#[derive(Debug, Parser, Clone)]
#[command(name = "wt-map-extractor", version, about = "War Thunder map extractor (Rust port)")]
pub struct Cli {
    /// Build all maps from client levels directory
    #[arg(long)]
    pub all: bool,

    /// Disable the local HTTP server (server starts by default)
    #[arg(long)]
    pub no_serve: bool,

    /// Export material textures (shared cache under maps/shared/mat)
    #[arg(long)]
    pub mat: bool,

    /// Export thumbnails (pending native thumbnail generator)
    #[arg(long)]
    pub thumbs: bool,

    /// Fast mode: skip expensive non-essential outputs (tile grid and rendinst)
    #[arg(long)]
    pub fast: bool,

    /// Disable compression for terrain_paint, heightmap, and normalmap outputs
    #[arg(long)]
    pub no_compress_map: bool,

    /// Optional map names (if omitted, auto-detect from for_test/levels)
    #[arg()]
    pub maps: Vec<String>,
}
