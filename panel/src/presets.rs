//! The kinds of workload an instance can run.

/// A preset an instance can be created with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// The hello world web server.
    #[default]
    HelloWorld,
    /// The benchmarker, which pushes against the limits of the instance.
    Benchmarker,
    /// An empty site the user of the instance fills with their own code.
    Site,
}

impl Preset {
    /// Every preset there is.
    pub const ALL: [Preset; 3] = [Preset::HelloWorld, Preset::Benchmarker, Preset::Site];

    /// The name of the preset in the database.
    pub fn slug(self) -> &'static str {
        match self {
            Preset::HelloWorld => "hello-world",
            Preset::Benchmarker => "benchmarker",
            Preset::Site => "site",
        }
    }

    /// The name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Preset::HelloWorld => "Hello World",
            Preset::Benchmarker => "Benchmarker",
            Preset::Site => "My site",
        }
    }

    /// A sentence about what the preset does.
    pub fn description(self) -> &'static str {
        match self {
            Preset::HelloWorld => {
                "A small web server that greets every visitor. Use it to see that \
                 an instance runs and answers."
            }
            Preset::Benchmarker => {
                "A web server that uses as much memory, processor time and disk as \
                 it can, so the limits of the instance can be checked."
            }
            Preset::Site => {
                "An empty website. You can write the html, the style and the \
                 script of it yourself, in the editor of the instance."
            }
        }
    }

    /// The binary that runs the preset.
    pub fn binary(self) -> &'static str {
        match self {
            Preset::HelloWorld | Preset::Site => "hello-server",
            Preset::Benchmarker => "benchmarker",
        }
    }

    /// Whether the user of the instance can edit the files of the site.
    pub fn is_editable(self) -> bool {
        matches!(self, Preset::Site | Preset::HelloWorld)
    }

    /// Whether the instance starts with the files of a site.
    ///
    /// The hello world preset keeps its own page until the user writes one,
    /// so it starts without any files.
    pub fn starts_with_files(self) -> bool {
        matches!(self, Preset::Site)
    }

    /// The preset with a given name, if there is one.
    pub fn from_slug(slug: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|preset| preset.slug() == slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_its_own_name() {
        for preset in Preset::ALL {
            assert_eq!(Preset::from_slug(preset.slug()), Some(preset));
        }
    }

    #[test]
    fn an_unknown_name_is_no_preset() {
        assert_eq!(Preset::from_slug("nope"), None);
        assert_eq!(Preset::from_slug(""), None);
    }

    #[test]
    fn the_site_runs_the_same_binary_as_the_hello_world() {
        assert_eq!(Preset::Site.binary(), Preset::HelloWorld.binary());
    }

    #[test]
    fn only_the_site_presets_can_be_edited() {
        for preset in Preset::ALL {
            assert_eq!(
                preset.is_editable(),
                preset == Preset::Site || preset == Preset::HelloWorld
            );
        }
    }

    #[test]
    fn the_names_are_lowercase_words() {
        for preset in Preset::ALL {
            assert!(preset
                .slug()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-'));
        }
    }
}
