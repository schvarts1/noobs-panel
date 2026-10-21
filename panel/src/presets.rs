//! The kinds of workload an instance can run.

/// A preset an instance can be created with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// The hello world web server.
    #[default]
    HelloWorld,
    /// The benchmarker, which pushes against the limits of the instance.
    Benchmarker,
}

impl Preset {
    /// Every preset there is.
    pub const ALL: [Preset; 2] = [Preset::HelloWorld, Preset::Benchmarker];

    /// The name of the preset in the database.
    pub fn slug(self) -> &'static str {
        match self {
            Preset::HelloWorld => "hello-world",
            Preset::Benchmarker => "benchmarker",
        }
    }

    /// The name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Preset::HelloWorld => "Hello World",
            Preset::Benchmarker => "Benchmarker",
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
        }
    }

    /// The binary that runs the preset.
    pub fn binary(self) -> &'static str {
        match self {
            Preset::HelloWorld => "hello-server",
            Preset::Benchmarker => "benchmarker",
        }
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
    fn the_names_are_lowercase_words() {
        for preset in Preset::ALL {
            assert!(preset
                .slug()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-'));
        }
    }
}
