// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit metadata-only local organization. Dialog IDs never follow mutable
//! row indices; destination pages and writes have their own bounded tickets.
use super::*;
use library::{OrganizationRequest as Request, OrganizationResult as ResultValue};
use oxplay_storage::{LocalPlaylist, PlaylistWindowPage};

#[derive(Clone, Copy)]
enum Kind {
    Destinations,
    Duplicate,
    Transfer(bool),
}
struct Pending {
    ticket: u64,
    generation: u64,
    context: CreateContext,
    kind: Kind,
}
struct Draft {
    source: LocalPlaylist,
    video: Option<VideoSummary>,
    context: CreateContext,
    destinations: Vec<LocalPlaylist>,
    page: CollectionWindow,
    selected: Option<usize>,
}
#[derive(Default)]
pub(super) struct State {
    serial: Cell<u64>,
    generation: Cell<u64>,
    draft: RefCell<Option<Draft>>,
    pending: RefCell<Option<Pending>>,
    names: Rc<slint::VecModel<slint::SharedString>>,
}
impl State {
    pub(super) fn pending(&self) -> bool {
        self.pending.borrow().is_some()
    }
}
fn message(app: &App, value: impl Into<slint::SharedString>) {
    app.global::<LibraryUi>()
        .set_organization_status(value.into());
}
pub(super) fn reset(app: &App, state: &UiState) {
    let state = &state.library_ui.organization;
    state.draft.borrow_mut().take();
    state.names.set_vec(Vec::new());
    let ui = app.global::<LibraryUi>();
    ui.set_organization(0);
    ui.set_organization_source("".into());
    ui.set_organization_video("".into());
    ui.set_organization_name("".into());
    ui.set_destination_index(-1);
    ui.set_destination_previous(false);
    ui.set_destination_next(false);
    ui.set_organization_status("".into());
}
fn valid(app: &App, state: &UiState, draft: &Draft) -> bool {
    context_matches(app, state, draft.context)
        && selected(app, state) == Some(draft.source.id)
        && !state.caption_cache.active()
        && app.global::<LibraryUi>().get_confirmation() == 0
}
fn begin(app: &App, state: &UiState, index: Option<i32>) -> bool {
    if app.get_page() != 1
        || app.global::<LibraryUi>().get_tab() != 0
        || state.library_ui.pending.get()
        || state.caption_cache.active()
        || !state.playback_preferences.ready()
        || app.global::<LibraryUi>().get_confirmation() != 0
        || state.library_ui.organization.draft.borrow().is_some()
    {
        return false;
    }
    let source = state
        .playlists
        .borrow()
        .get(app.get_selected_playlist() as usize)
        .cloned();
    let Some(source) = source else { return false };
    let video = if let Some(index) = index {
        if state.library_ui.published_route.borrow().as_ref() != Some(&current_route(app, state)) {
            return false;
        }
        match state.library_ui.items.borrow().get(index as usize) {
            Some(Item::Video(video)) => Some(video.clone()),
            _ => return false,
        }
    } else {
        None
    };
    let organization = &state.library_ui.organization;
    let Some(generation) = organization.generation.get().checked_add(1) else {
        return false;
    };
    organization.generation.set(generation);
    cancel_name(app, state);
    let ui = app.global::<LibraryUi>();
    ui.set_organization_source(source.name.as_str().into());
    ui.set_organization_video(
        video
            .as_ref()
            .map_or("", |video| video.title.as_str())
            .into(),
    );
    ui.set_organization_name("".into());
    ui.set_organization(if video.is_some() { 2 } else { 1 });
    *organization.draft.borrow_mut() = Some(Draft {
        source,
        video,
        context: create_context(app, state),
        destinations: Vec::new(),
        page: CollectionWindow::default(),
        selected: None,
    });
    if index.is_some() {
        destinations(app, state, 0);
    } else {
        message(
            app,
            "Choose a name for the new local playlist. Saved metadata is copied; media is not downloaded.",
        );
    }
    true
}
fn enqueue(app: &App, state: &UiState, action: Request, kind: Kind) -> bool {
    let organization = &state.library_ui.organization;
    if organization.pending() {
        return false;
    }
    let context = organization
        .draft
        .borrow()
        .as_ref()
        .filter(|draft| valid(app, state, draft))
        .map(|draft| draft.context);
    let Some(context) = context else {
        reset(app, state);
        status(
            app,
            "The local playlist context changed. Open the action again.",
        );
        return false;
    };
    let Some(ticket) = organization.serial.get().checked_add(1) else {
        return false;
    };
    if !submit(app, state, library::Request::Organize { ticket, action }) {
        message(
            app,
            "The local library is busy. Nothing was queued; try again.",
        );
        return false;
    }
    organization.serial.set(ticket);
    *organization.pending.borrow_mut() = Some(Pending {
        ticket,
        generation: organization.generation.get(),
        context,
        kind,
    });
    message(
        app,
        match kind {
            Kind::Destinations => "Loading local destinations…",
            Kind::Duplicate => "Duplicating saved metadata…",
            Kind::Transfer(false) => "Copying saved video…",
            Kind::Transfer(true) => "Moving saved video…",
        },
    );
    true
}
fn destinations(app: &App, state: &UiState, direction: i32) {
    let query = {
        let draft = state.library_ui.organization.draft.borrow();
        let Some(draft) = draft.as_ref().filter(|draft| draft.video.is_some()) else {
            return;
        };
        match direction {
            0 => Some(draft.page.current),
            1 => draft.page.previous,
            2 => draft.page.next,
            _ => None,
        }
    };
    if let Some(query) = query {
        enqueue(app, state, Request::Destinations(query), Kind::Destinations);
    }
}
fn publish_destinations(app: &App, state: &UiState, page: PlaylistWindowPage) {
    let organization = &state.library_ui.organization;
    let (names, selected, has_previous, has_next) = {
        let mut draft = organization.draft.borrow_mut();
        let Some(draft) = draft.as_mut() else { return };
        let previous_id = draft
            .selected
            .and_then(|index| draft.destinations.get(index))
            .map(|item| item.id);
        draft.destinations = page
            .items
            .into_iter()
            .filter(|item| item.id != draft.source.id)
            .collect();
        draft.selected =
            previous_id.and_then(|id| draft.destinations.iter().position(|item| item.id == id));
        draft.page = CollectionWindow {
            current: page.current,
            previous: page.previous,
            next: page.next,
        };
        let names: Vec<slint::SharedString> = draft
            .destinations
            .iter()
            .map(|item| item.name.as_str().into())
            .collect();
        (
            names,
            draft.selected.map_or(-1, |index| index as i32),
            draft.page.previous.is_some(),
            draft.page.next.is_some(),
        )
    };
    organization.names.set_vec(names);
    let ui = app.global::<LibraryUi>();
    ui.set_destination_index(selected);
    ui.set_destination_previous(has_previous);
    ui.set_destination_next(has_next);
    message(
        app,
        "Choose another local playlist. Copy keeps this entry here; Move removes it only after the destination is saved.",
    );
}
pub(super) fn receive(
    app: &App,
    state: &UiState,
    ticket: u64,
    result: std::result::Result<ResultValue, String>,
) {
    let organization = &state.library_ui.organization;
    let pending = {
        let mut pending = organization.pending.borrow_mut();
        if pending
            .as_ref()
            .is_some_and(|pending| pending.ticket == ticket)
        {
            pending.take()
        } else {
            None
        }
    };
    let Some(pending) = pending else { return };
    complete(app, state);
    let live = organization.generation.get() == pending.generation
        && organization
            .draft
            .borrow()
            .as_ref()
            .is_some_and(|draft| valid(app, state, draft));
    match (pending.kind, result) {
        (Kind::Destinations, Ok(ResultValue::Destinations(page))) => {
            if live {
                publish_destinations(app, state, page);
            }
        }
        (Kind::Duplicate, Ok(ResultValue::Duplicated(id))) => {
            crate::home_ui::changed(app, state);
            reset(app, state);
            status(app, "Playlist duplicated on this device.");
            if live && context_matches(app, state, pending.context) {
                if !reveal_created(app, state, id) {
                    status(
                        app,
                        "Playlist duplicated, but its page could not be loaded. Reopen local playlists.",
                    );
                }
            } else {
                collections(app, state);
            }
        }
        (Kind::Transfer(move_item), Ok(ResultValue::Transferred)) => {
            crate::home_ui::changed(app, state);
            reset(app, state);
            status(
                app,
                if move_item {
                    "Saved video moved between local playlists."
                } else {
                    "Saved video copied to the local playlist."
                },
            );
            if live {
                refresh(app, state);
            }
        }
        (_, Err(error)) => {
            if live {
                message(
                    app,
                    format!("{error} No change was committed. You can try again."),
                );
            } else {
                status(app, error);
            }
        }
        _ => {
            if live {
                message(
                    app,
                    "Unexpected local-library response. Close this action and try again.",
                );
            }
        }
    }
}
pub(super) fn bind(app: &App, state: &Rc<UiState>) {
    app.global::<LibraryUi>()
        .set_destinations(slint::ModelRc::from(
            state.library_ui.organization.names.clone(),
        ));
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>()
        .on_begin_duplicate(move || weak.upgrade().is_some_and(|app| begin(&app, &s, None)));
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_begin_transfer(move |index| {
        weak.upgrade()
            .is_some_and(|app| begin(&app, &s, Some(index)))
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_cancel_organization(move || {
        if let Some(app) = weak.upgrade()
            && !s.library_ui.organization.pending()
        {
            reset(&app, &s);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>()
        .on_destination_page(move |direction| {
            if let Some(app) = weak.upgrade() {
                destinations(&app, &s, direction);
            }
        });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>()
        .on_choose_destination(move |index| {
            let Some(app) = weak.upgrade() else { return };
            if s.library_ui.pending.get() {
                return;
            }
            let accepted = {
                let mut draft = s.library_ui.organization.draft.borrow_mut();
                let Some(draft) = draft.as_mut().filter(|draft| valid(&app, &s, draft)) else {
                    return;
                };
                let Ok(index) = usize::try_from(index) else {
                    return;
                };
                if index >= draft.destinations.len() {
                    return;
                }
                draft.selected = Some(index);
                index as i32
            };
            app.global::<LibraryUi>().set_destination_index(accepted);
        });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>().on_duplicate(move |name| {
        let Some(app) = weak.upgrade() else { return };
        let name = match playlist_name(&name) {
            Ok(name) => name,
            Err(error) => {
                message(&app, error);
                return;
            }
        };
        let source = s
            .library_ui
            .organization
            .draft
            .borrow()
            .as_ref()
            .filter(|draft| draft.video.is_none())
            .map(|draft| draft.source.id);
        if let Some(source) = source {
            enqueue(
                &app,
                &s,
                Request::Duplicate { source, name },
                Kind::Duplicate,
            );
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.global::<LibraryUi>()
        .on_transfer_item(move |move_item| {
            let Some(app) = weak.upgrade() else { return };
            let request = {
                let draft = s.library_ui.organization.draft.borrow();
                let Some(draft) = draft.as_ref() else { return };
                let Some(video) = draft.video.as_ref() else {
                    return;
                };
                let Some(destination) = draft
                    .selected
                    .and_then(|index| draft.destinations.get(index))
                else {
                    message(&app, "Choose a destination playlist.");
                    return;
                };
                Request::Transfer {
                    source: draft.source.id,
                    destination: destination.id,
                    video: video.id.clone(),
                    move_item,
                }
            };
            enqueue(&app, &s, request, Kind::Transfer(move_item));
        });
}
