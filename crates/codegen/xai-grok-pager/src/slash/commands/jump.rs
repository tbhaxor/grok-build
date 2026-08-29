//! `/jump` — open the turn picker, or `/jump N` to jump straight to turn N.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand, slash_meta};
use crate::slash::{ModeSupport, Remedy};

pub struct JumpCommand;

impl SlashCommand for JumpCommand {
    slash_meta! {
        name: "jump",
        description: "Jump to a turn (/jump [N])",
        usage: "/jump [N]",
        takes_args: true,
        args_required: false,
        session_scoped: true,
        mode_support: ModeSupport::FullscreenOnly(Remedy::SwitchMode {
            why: "minimal scrolls with your terminal's native scrollback",
        }),
        arg_placeholder: "[N]",
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        match parse_jump_args(args) {
            Ok(None) => CommandResult::Action(Action::JumpShowPicker),
            Ok(Some(turn)) => CommandResult::Action(Action::JumpToTurn(turn)),
            Err(msg) => CommandResult::Error(msg),
        }
    }
}

/// Parse `/jump` args: empty → picker; a 1-based turn number → direct jump.
fn parse_jump_args(args: &str) -> Result<Option<usize>, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed.parse::<usize>() {
        Ok(0) => Err("Turn number must be 1 or greater".to_string()),
        Ok(n) => Ok(Some(n)),
        Err(_) => Err("Usage: /jump [N] where N is a turn number (e.g. /jump 10)".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::model_state::ModelState;
    use crate::app::actions::Action;
    use crate::app::bundle::BundleState;
    use crate::settings::PagerLocalSnapshot;

    static DEFAULT_BUNDLE_STATE: BundleState = BundleState {
        has_cache: false,
        version: String::new(),
        personas: Vec::new(),
        roles: Vec::new(),
        agents: Vec::new(),
        skills: Vec::new(),
        persona_details: Vec::new(),
        role_details: Vec::new(),
    };

    fn make_ctx(models: &ModelState) -> CommandExecCtx<'_> {
        CommandExecCtx {
            models,
            session_id: None,
            bundle_state: &DEFAULT_BUNDLE_STATE,
            screen_mode: crate::app::ScreenMode::Fullscreen,
            billing_surface_visible: true,
            usage_command_visible: true,
            pager_state: PagerLocalSnapshot::default(),
        }
    }

    #[test]
    fn jump_no_args_opens_picker() {
        let models = ModelState::default();
        let mut ctx = make_ctx(&models);
        let result = JumpCommand.run(&mut ctx, "");
        assert!(matches!(
            result,
            CommandResult::Action(Action::JumpShowPicker)
        ));
    }

    #[test]
    fn jump_with_number_jumps_directly() {
        let models = ModelState::default();
        let mut ctx = make_ctx(&models);
        let result = JumpCommand.run(&mut ctx, "10");
        assert!(matches!(
            result,
            CommandResult::Action(Action::JumpToTurn(10))
        ));
    }

    #[test]
    fn jump_rejects_zero() {
        let models = ModelState::default();
        let mut ctx = make_ctx(&models);
        let result = JumpCommand.run(&mut ctx, "0");
        assert!(matches!(result, CommandResult::Error(msg) if msg.contains("1 or greater")));
    }

    #[test]
    fn jump_rejects_non_numeric() {
        let models = ModelState::default();
        let mut ctx = make_ctx(&models);
        let result = JumpCommand.run(&mut ctx, "abc");
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn parse_trims_whitespace() {
        assert_eq!(parse_jump_args("  3  ").unwrap(), Some(3));
        assert_eq!(parse_jump_args("   ").unwrap(), None);
    }
}
