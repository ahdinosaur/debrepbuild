mod build;
mod compress;
mod download;
mod generate;
mod pool;
mod prepare;
mod version;

use config::Config;
use rayon;
use std::{env, fs, io};
use std::path::PathBuf;
use std::process::exit;

pub enum Packages<'a> {
    All,
    Select(&'a [&'a str], bool)
}

pub struct Repo<'a> {
    config: Config,
    packages: Packages<'a>
}

impl<'a> Repo<'a> {
    pub fn prepare(config: Config, packages: Packages<'a>) -> Repo<'a> {
        if let Err(why) = prepare::create_missing_directories() {
            error!("unable to create directories in current directory: {}", why);
            exit(1);
        }

        Repo { config, packages }
    }

    pub fn clean(self) -> Self {
        if let Err(why) = prepare::package_cleanup(&self.config) {
            error!("failed to clean up file: {}", why);
            exit(1);
        }
        self
    }

    pub fn download(self) -> Self {
        match self.packages {
            Packages::All => download::all(&self.config),
            Packages::Select(ref packages, _) => {
                download::packages(&self.config, packages)
            }
        }

        self
    }

    pub fn build(self) -> Self {
        match self.packages {
            Packages::All => build::all(&self.config),
            Packages::Select(ref packages, force) => {
                build::packages(&self.config, packages, force)
            }
        }

        self
    }

    pub fn generate(self) {
        if let Err(why) = generate_release_files(&self.config) {
            error!("failed to generate dist files: {}", why);
            exit(1);
        }
    }

    pub fn remove(self) -> Self {
        if let Packages::Select(ref packages, _) = self.packages {
            if let Err(why) = prepare::remove(packages, &self.config.archive) {
                error!("failed to remove file: {}", why);
                exit(1);
            }
        }

        self
    }
}

#[derive(Debug, Fail)]
pub enum ReleaseError {
    #[fail(display = "failed to generate release files for binaries: {}", why)]
    Binary { why: io::Error },
    #[fail(display = "failed to generate contents for binaries: {}", why)]
    Contents { why: io::Error },
    #[fail(display = "failed to generate source index: {}", why)]
    Source { why: io::Error },
    #[fail(display = "failed to generate dist release files for {}: {}", archive, why)]
    Dists { archive: String, why: io::Error },
    #[fail(display = "failed to generate InRelease file: {}", why)]
    InRelease { why: io::Error },
    #[fail(display = "failed to generate Release.gpg file: {}", why)]
    ReleaseGPG { why: io::Error },
}

/// Generate the dist release files from the existing binary and source files.
fn generate_release_files(sources: &Config) -> Result<(), ReleaseError> {
    env::set_current_dir("repo").expect("unable to switch dir to repo");
    let base = ["dists/", &sources.archive].concat();
    let pool = ["pool/", &sources.archive, "/main"].concat();
    let _ = fs::create_dir_all(&base);

    let release = PathBuf::from([&base, "/Release"].concat());
    let in_release = PathBuf::from([&base, "/InRelease"].concat());
    let release_gpg = PathBuf::from([&base, "/Release.gpg"].concat());

    let mut binary_result = Ok(());
    let mut sources_result = Ok(());
    let mut contents_result = Ok(());
    let mut dists_result = Ok(());

    rayon::scope(|s| {
        s.spawn(|_| {
            binary_result = generate::binary_files(sources, &base, &pool).map_err(|why| ReleaseError::Binary { why });
            dists_result = generate::dists_release(sources, &base).map_err(|why| ReleaseError::Dists {
                archive: sources.archive.clone(),
                why,
            });
        });
        s.spawn(|_| {
            sources_result = generate::sources_index(&base, &pool).map_err(|why| ReleaseError::Source { why });
        });
        s.spawn(|_| {
            contents_result = generate::contents(&base, &pool).map_err(|why| ReleaseError::Contents { why });
        });
    });

    binary_result.and(sources_result).and(contents_result).and(dists_result)?;

    let (inrelease, release) = rayon::join(
        || {
            generate::gpg_in_release(&sources.email, &release, &in_release)
                .map_err(|why| ReleaseError::InRelease { why })
        },
        || {
            generate::gpg_release(&sources.email, &release, &release_gpg)
            .map_err(|why| ReleaseError::ReleaseGPG { why })
        }
    );

    inrelease.and(release)
}
