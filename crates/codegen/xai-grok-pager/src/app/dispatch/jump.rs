//! `/jump` picker dispatchers: pure client-side turn navigation.

use crate::app::actions::Effect;
use crate::app::app_view::{ActiveView, AppView};
use crate::scrollback::entry::EntryId;
use crate::views::jump::{JumpRestore, JumpState};

pub(super) fn dispatch_jump_show_picker(app: &mut AppView) -> Vec<Effect> {
    let ActiveView::Agent(id) = app.active_view else {
        return vec![];
    };
    let Some(agent) = app.agents.get_mut(&id) else {
        return vec![];
    };
    // Refuse if another prompt overlay owns the input slot (rewind, inline-edit, /btw, or a pending permission/question/cancel-turn/plan overlay)
    // An opened picker would be hidden but still eat input
    if agent.jump_slot_taken() {
        return vec![];
    }

    let entries = agent.scrollback.timeline_entries();
    if entries.len() < 2 {
        app.show_toast("Nothing to jump to yet");
        return vec![];
    }

    let restore = JumpRestore {
        bookmark: agent.scrollback.capture_scroll_bookmark(),
        selected: agent.scrollback.selected(),
        follow_mode: agent.scrollback.is_follow_mode(),
    };
    // Open on the session-info turn currently at the viewport top (picker
    // rows are keyed by that index, not raw scrollback position).
    let selected = agent
        .scrollback
        .active_display_turn()
        .and_then(|display| entries.iter().position(|e| e.turn_idx + 1 == display))
        .or_else(|| {
            let sb_idx = agent.scrollback.active_turn_for_viewport()?;
            let prompt_idx = agent.scrollback.turn(sb_idx)?.prompt_index;
            let id = agent.scrollback.iter_entries().nth(prompt_idx)?.0;
            entries.iter().position(|e| e.prompt_entry_id == id)
        })
        .unwrap_or(entries.len() - 1)
        .min(entries.len() - 1);

    let preview_id = entries[selected].prompt_entry_id;
    agent.jump_state = Some(JumpState {
        entries,
        selected,
        restore,
    });
    // The same top anchor that cursor moves preview and that Enter lands on
    if let Some(idx) = agent.scrollback.index_of_id(preview_id) {
        agent.scrollback.scroll_to_entry_top(idx);
    }
    vec![]
}

pub(super) fn dispatch_jump_picker_select(app: &mut AppView, prompt_id: EntryId) -> Vec<Effect> {
    let ActiveView::Agent(id) = app.active_view else {
        return vec![];
    };
    let Some(agent) = app.agents.get_mut(&id) else {
        return vec![];
    };
    let Some(js) = agent.jump_state.take() else {
        return vec![];
    };
    // The stable id resolves at the boundary; it fails only if the prompt was removed (async clear/rewind) while the picker was open
    // Restore the captured viewport so a failed jump never strands the transcript at the last preview scroll
    if !agent.scrollback.jump_to_entry(prompt_id) {
        agent.restore_jump_viewport(js.restore);
    } else {
        agent.scrollback.start_jump_flash();
    }
    vec![]
}

pub(super) fn dispatch_jump_dismiss(app: &mut AppView) -> Vec<Effect> {
    let ActiveView::Agent(id) = app.active_view else {
        return vec![];
    };
    let Some(agent) = app.agents.get_mut(&id) else {
        return vec![];
    };
    agent.dismiss_jump_picker();
    vec![]
}

/// `/jump N`: jump to the 1-based turn number. Out-of-range turns toast and
/// leave the viewport alone. Dismisses an open jump picker first so the two
/// navigation UIs never fight.
pub(super) fn dispatch_jump_to_turn(app: &mut AppView, turn_number: usize) -> Vec<Effect> {
    let ActiveView::Agent(id) = app.active_view else {
        return vec![];
    };
    let Some(agent) = app.agents.get_mut(&id) else {
        return vec![];
    };
    // A leftover picker would still own keys/wheel after a direct jump.
    agent.dismiss_jump_picker();

    // Display is 1-based and matches `/session-info` / hover (shell prompt index).
    if !agent.scrollback.jump_to_display_turn(turn_number) {
        app.show_toast(&format!("Turn {turn_number} doesn't exist"));
    } else {
        agent.scrollback.start_jump_flash();
    }
    vec![]
}
