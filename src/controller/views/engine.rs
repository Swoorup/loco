use std::path::{Path, PathBuf};

use super::tera_builtins;
use crate::{controller::views::ViewRenderer, Error, Result};
use serde::Serialize;

pub static DEFAULT_ASSET_FOLDER: &str = "assets";

type TeraPostProcessor = std::sync::Arc<dyn Fn(&mut tera::Tera) -> Result<()> + Send + Sync>;

#[derive(Clone)]
pub struct TeraView {
    #[cfg(debug_assertions)]
    pub tera: std::sync::Arc<std::sync::Mutex<tera::Tera>>,

    #[cfg(not(debug_assertions))]
    pub tera: tera::Tera,

    #[cfg(debug_assertions)]
    pub view_dir: String,

    pub tera_post_process: Option<TeraPostProcessor>,

    pub default_context: tera::Context,
}

impl std::fmt::Debug for TeraView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut f = f.debug_struct("TeraView");
        f.field("tera", &self.tera);
        #[cfg(debug_assertions)]
        let f = f.field("view_dir", &self.view_dir);
        let f = f
            .field(
                "tera_post_process",
                if self.tera_post_process.is_some() {
                    &Some("Fn")
                } else {
                    &None::<&'static str>
                },
            )
            .field("default_context", &self.default_context);
        f.finish()
    }
}

impl TeraView {
    /// Create a Tera view engine
    ///
    /// # Errors
    ///
    /// This function will return an error if building fails
    pub fn build() -> Result<Self> {
        Self::from_custom_dir(&PathBuf::from(DEFAULT_ASSET_FOLDER).join("views"))
    }

    /// Attach the Tera view engine with a post-processing function for subsequent instantiation.
    ///
    /// The post-processing function is also run during the call to this method.
    ///
    /// # Errors
    ///
    /// This function will return an error if the post-processing function fails
    pub fn post_process(
        mut self,
        post_process: impl Fn(&mut tera::Tera) -> Result<()> + Send + Sync + 'static,
    ) -> Result<Self> {
        {
            #[cfg(debug_assertions)]
            let engine = &mut *self.tera.lock().unwrap();

            #[cfg(not(debug_assertions))]
            let engine = &mut self.tera;

            post_process(engine)?;
        }

        self.tera_post_process = Some(std::sync::Arc::new(post_process));
        Ok(self)
    }

    /// Create a new Tera instance from a directory path
    ///
    /// # Errors
    ///
    /// This function will return an error if building fails
    fn create_tera_instance<P: AsRef<Path>>(path: P) -> Result<tera::Tera> {
        let mut tera = tera::Tera::new(
            path.as_ref()
                .join("**")
                .join("*.html")
                .to_str()
                .ok_or_else(|| Error::string("invalid blob"))?,
        )?;
        tera_builtins::filters::register_filters(&mut tera);
        Ok(tera)
    }

    /// Create a Tera view engine from a custom directory
    ///
    /// # Errors
    ///
    /// This function will return an error if building fails
    pub fn from_custom_dir<P: AsRef<Path>>(path: &P) -> Result<Self> {
        if !path.as_ref().exists() {
            return Err(Error::string(&format!(
                "missing views directory: `{}`",
                path.as_ref().display()
            )));
        }

        let tera = Self::create_tera_instance(path.as_ref())?;
        let ctx = tera::Context::default();
        Ok(Self {
            #[cfg(debug_assertions)]
            view_dir: path.as_ref().to_string_lossy().to_string(),
            tera_post_process: None,
            #[cfg(debug_assertions)]
            tera: std::sync::Arc::new(std::sync::Mutex::new(tera)),
            #[cfg(not(debug_assertions))]
            tera: tera,
            default_context: ctx,
        })
    }
}

impl ViewRenderer for TeraView {
    fn render<S: Serialize>(&self, key: &str, data: S) -> Result<String> {
        let context = tera::Context::from_serialize(data)?;

        #[cfg(debug_assertions)]
        {
            tracing::debug!(key = key, "Tera rendering in non-optimized debug mode");
            let mut tera = Self::create_tera_instance(&self.view_dir)?;
            if let Some(post_process) = self.tera_post_process.as_deref() {
                post_process(&mut tera)?;
            }
            Ok(tera.render(key, &context)?)
        }

        #[cfg(not(debug_assertions))]
        Ok(self.tera.render(key, &context)?)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use tree_fs;

    use super::*;
    #[test]
    fn can_render_view() {
        let tree_fs = tree_fs::TreeBuilder::default()
            .add_file("template/test.html", "generate test.html file: {{foo}}")
            .add_file("template/test2.html", "generate test2.html file: {{bar}}")
            .create()
            .unwrap();

        let v = TeraView::from_custom_dir(&tree_fs.root).unwrap();

        assert_eq!(
            v.render("template/test.html", json!({"foo": "foo-txt"}))
                .unwrap(),
            "generate test.html file: foo-txt"
        );

        assert_eq!(
            v.render("template/test2.html", json!({"bar": "bar-txt"}))
                .unwrap(),
            "generate test2.html file: bar-txt"
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn template_inheritance_hot_reload() {
        let tree_fs = tree_fs::TreeBuilder::default()
            .add_file(
                "template/base.html",
                r"<!DOCTYPE html>
            <html>
            <head>
                <title>{% block title %}Default Title{% endblock %}</title>
            </head>
            <body>
                <header>Base Header v1: {{ 1 | hello }}</header>
                {% block content %}
                Default content
                {% endblock %}
                <footer>Base Footer</footer>
            </body>
            </html>",
            )
            .add_file(
                "template/child.html",
                r"{% extends 'template/base.html' %}
            {% block title %}Child Page{% endblock %}
            {% block content %}
            <div>Child content</div>
            {% endblock %}",
            )
            .create()
            .unwrap();

        let tree_dir = tree_fs.root.clone();
        let v = TeraView::from_custom_dir(&tree_fs.root)
            .unwrap()
            .post_process(|tera| {
                tera.register_filter("hello", |value: &Value, _: &HashMap<String, Value>| {
                    Ok(format!("Hello World v{value}").into())
                });
                Ok(())
            })
            .unwrap();

        // Initial render should have the original header from base template
        let initial_render = v.render("template/child.html", json!({})).unwrap();
        assert!(initial_render.contains("Base Header v1: Hello World v1"));
        assert!(initial_render.contains("Child Page"));
        assert!(initial_render.contains("Child content"));

        // Now modify the base template to change the header
        let updated_base = r"<!DOCTYPE html>
<html>
<head>
    <title>{% block title %}Default Title{% endblock %}</title>
</head>
<body>
    <header>Base Header v2: {{ 2 | hello }}</header>
    {% block content %}
    Default content
    {% endblock %}
    <footer>Base Footer</footer>
</body>
</html>";

        // Update the base template file
        std::fs::write(
            Path::new(&tree_dir).join("template").join("base.html"),
            updated_base,
        )
        .unwrap();

        // Render again - should have the updated header due to hot reload
        let updated_render = v.render("template/child.html", json!({})).unwrap();
        assert!(updated_render.contains("Base Header v2: Hello World v2")); // Should have changed
        assert!(updated_render.contains("Child Page")); // Should be the same
        assert!(updated_render.contains("Child content")); // Should be the same
    }
}
