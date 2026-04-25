mod cli;
mod config;
mod dxp_index;
mod export;
mod extract;
mod heightmap;
mod inspect;
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
use crate::util::set_oodle_dll_path;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let serve = !cli.no_serve;
    let will_build = cli.all || !cli.maps.is_empty() || (!serve && !cli.inspect);

    let cfg = AppConfig::load(".")?;
    // Register oo2core DLL path with the decompressor before any map processing.
    set_oodle_dll_path(cfg.oo2core_dll.as_deref());
    let pipe = Pipeline::new(cfg.clone());

    let opts = BuildOptions {
        local_output: cli.local,
        export_mat: cli.mat || cli.debug || will_build,
        // Preconverting the shared material cache decodes every DxP pack in
        // the datamine (tens of GB of RAM/disk). Only do it when the user
        // explicitly passes --preconvert-mat. `--mat` / `--debug` / `--all`
        // now rely on the per-map streaming extraction path.
        preconvert_mat: cli.preconvert_mat,
        export_thumbs: cli.thumbs || cli.debug,
        clean_maps_on_all: true,
        skip_tile_grid: cli.fast || cli.skip_tile_grid,
        skip_rendinst: cli.fast || cli.skip_rendinst,
        compress_maps: !cli.no_compress_map,
        sun_azimuth: cli.sun_azimuth,
        sun_elevation: cli.sun_elevation,
        sun_strength: cli.sun_strength,
        debug: cli.debug,
    };

    if cli.inspect {
        if cli.maps.is_empty() {
            anyhow::bail!("--inspect requires at least one map name, e.g. `--inspect air_israel avg_japan`");
        }
        for map in &cli.maps {
            match pipe.inspect_map(map) {
                Ok(path) => println!("Wrote {}", path.display()),
                Err(e) => eprintln!("  ! inspect failed for {map}: {e:#}"),
            }
        }
        return Ok(());
    }

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
        if opts.export_mat && opts.preconvert_mat {
            pipe.preconvert_shared_materials(true)
                .context("Failed preconverting materials")?;
        }
        for map in &cli.maps {
            pipe.build_one(Some(map), &opts)
                .with_context(|| format!("Failed building map '{map}'"))?;
        }
    } else if !serve {
        // No --all, no map names, --no-serve → auto-detect from for_test/levels
        if opts.export_mat && opts.preconvert_mat {
            pipe.preconvert_shared_materials(true)
                .context("Failed preconverting materials")?;
        }
        pipe.build_one(None, &opts)?;
    }

    if serve {
        let url = "http://127.0.0.1:8000";
        println!("Opening browser at {}", url);
        let _ = webbrowser::open(url);
        server::serve(&cfg.project_root, "127.0.0.1:8000")?;
    }

    Ok(())
}
