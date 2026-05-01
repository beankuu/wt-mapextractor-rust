mod cli;
mod config;
mod dxp_index;
mod export;
mod extract;
mod heightmap;
mod landclass;
mod missions;
mod paint;
mod post;
mod pipeline;
mod progress;
mod rendinst;
mod server;
mod util;

use anyhow::{Context, Result};
use clap::Parser;

use crate::cli::Cli;
use crate::config::AppConfig;
use crate::pipeline::{BuildOptions, Pipeline};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let serve = !cli.no_serve;

    let cfg = AppConfig::load(".")?;
    let pipe = Pipeline::new(cfg.clone());

    let opts = BuildOptions {
        export_mat: cli.mat,
        export_thumbs: cli.thumbs,
        clean_maps_on_all: true,
        skip_tile_grid: cli.fast,
        skip_rendinst: cli.fast,
        compress_maps: !cli.no_compress_map,
        // Fixed bake-time sun; live user controls are handled in the viewer UI.
        sun_azimuth: 30.0,
        sun_elevation: 50.0,
        sun_strength: 0.5,
    };

    if cli.all {
        // build_all returns Err if any maps failed; still run serve afterwards
        let build_result = pipe.build_all(&opts);
        if serve {
            if let Err(ref e) = build_result {
                eprintln!("  Warning: batch build had failures: {e:#}");
            }
        } else {
            build_result?;
        }
    } else if !cli.maps.is_empty() {
        for map in &cli.maps {
            pipe.build_one(Some(map), &opts)
                .with_context(|| format!("Failed building map '{map}'"))?;
        }
    } else if !serve {
        // No --all, no map names, --no-serve → auto-detect from for_test/levels
        pipe.build_one(None, &opts)?;
    }

    if serve {
        // If specific map(s) were built, jump straight to the first one's
        // viewer page instead of the index. Otherwise (--all, or no args)
        // open the index so the user can pick a map.
        let base = "http://127.0.0.1:8000";
        let url = if !cli.all && !cli.maps.is_empty() {
            format!("{}/src/viewer.html?map={}", base, cli.maps[0])
        } else {
            base.to_string()
        };
        println!("Opening browser at {}", url);
        let _ = webbrowser::open(&url);
        server::serve(&cfg.project_root, "127.0.0.1:8000")?;
    }

    Ok(())
}
