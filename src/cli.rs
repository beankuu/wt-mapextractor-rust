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

    /// Build into ./viewer_data instead of ./maps/<map>/viewer_data
    #[arg(long)]
    pub local: bool,

    /// Export material textures (shared cache under maps/shared/mat)
    #[arg(long)]
    pub mat: bool,

    /// Preconvert all available DxP materials once at startup into maps/shared/mat.
    #[arg(long)]
    pub preconvert_mat: bool,

    /// Export thumbnails (pending native thumbnail generator)
    #[arg(long)]
    pub thumbs: bool,

    /// Debug bundle: enables --thumbs + --mat, and emits extra diagnostics
    /// (per-map landclass BLK dump, synth `lc_N` source cells, per-cell
    /// splat weight manifest for per-pixel hover).
    #[arg(long)]
    pub debug: bool,

    /// Fast mode: skip expensive non-essential outputs (tile grid and rendinst)
    #[arg(long)]
    pub fast: bool,

    /// Skip tile_grid generation
    #[arg(long)]
    pub skip_tile_grid: bool,

    /// Skip rendinst metadata extraction
    #[arg(long)]
    pub skip_rendinst: bool,

    /// Disable compression for terrain_paint, heightmap, and normalmap outputs
    #[arg(long)]
    pub no_compress_map: bool,

    /// Sun azimuth in degrees (0=N, 90=E, 180=S, 270=W)
    #[arg(long, default_value_t = 30.0)]
    pub sun_azimuth: f64,

    /// Sun elevation in degrees (0=horizon, 90=overhead)
    #[arg(long, default_value_t = 50.0)]
    pub sun_elevation: f64,

    /// Sun diffuse intensity (0.0 - 1.0)
    #[arg(long, default_value_t = 0.5)]
    pub sun_strength: f64,

    /// Inspect .bin files: dump every known structure plus unknown regions
    /// into ./maps/<map>/inspect.txt. Does not produce a viewer build.
    #[arg(long)]
    pub inspect: bool,

    /// Optional map names (if omitted, auto-detect from for_test/levels)
    #[arg()]
    pub maps: Vec<String>,
}
