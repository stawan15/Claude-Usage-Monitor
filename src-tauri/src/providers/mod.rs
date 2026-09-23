mod claude;
mod codex;
mod gemini;
mod opencode;
mod tail;

use crate::model::Provider;

pub fn all() -> Vec<Box<dyn Provider>> {
    vec![
        Box::<claude::ClaudeCode>::default(),
        Box::<codex::Codex>::default(),
        Box::<opencode::OpenCode>::default(),
        Box::<gemini::Gemini>::default(),
    ]
}
