use gio::prelude::*;
use gio::SimpleActionGroup;
use std::cell::Ref;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

use crate::app::components::{labels, PlaylistModel, SelectionTool, SelectionToolsModel};
use crate::app::models::SongBatch;
use crate::app::models::SongDescription;
use crate::app::models::SongModel;
use crate::app::state::PlaylistSource;
use crate::app::state::{PlaybackAction, SelectionAction, SelectionContext, SelectionState};
use crate::app::BrowserAction;
use crate::app::BrowserEvent;
use crate::app::{ActionDispatcher, AppAction, AppEvent, AppModel, AppState, ListDiff};
use crate::{api::SpotifyApiClient, app::components::SimpleSelectionTool};

pub struct SavedTracksModel {
    app_model: Rc<AppModel>,
    dispatcher: Box<dyn ActionDispatcher>,
}

impl SavedTracksModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Box<dyn ActionDispatcher>) -> Self {
        Self {
            app_model,
            dispatcher,
        }
    }

    fn state(&self) -> Ref<'_, AppState> {
        self.app_model.get_state()
    }

    fn songs(&self) -> Option<impl Deref<Target = Vec<SongDescription>> + '_> {
        self.app_model
            .map_state_opt(|s| Some(&s.browser.home_state()?.saved_tracks))
    }

    pub fn load_more(&self) -> Option<()> {
        let batch = self
            .state()
            .browser
            .home_state()?
            .last_saved_tracks_batch
            .next()?;
        let api = self.app_model.get_spotify();
        let batch_size = batch.batch_size;
        let next_offset = batch.offset;

        self.dispatcher
            .call_spotify_and_dispatch(move || async move {
                api.get_saved_tracks(next_offset, batch_size)
                    .await
                    .map(move |song_batch| BrowserAction::AppendSavedTracks(song_batch).into())
            });

        Some(())
    }
}

impl PlaylistModel for SavedTracksModel {
    fn current_song_id(&self) -> Option<String> {
        self.app_model
            .get_state()
            .playback
            .current_song_id()
            .cloned()
    }

    fn play_song(&self, id: &str) {
        let source = Some(PlaylistSource::SavedTracks);
        if let Some(home_state) = self.app_model.get_state().browser.home_state() {
            self.dispatcher.dispatch(
                PlaybackAction::LoadPagedSongs(
                    source,
                    SongBatch {
                        batch: home_state.last_saved_tracks_batch,
                        songs: home_state.saved_tracks.clone(),
                    },
                )
                .into(),
            );
        }

        self.dispatcher
            .dispatch(PlaybackAction::Load(id.to_string()).into());
    }

    fn diff_for_event(&self, event: &AppEvent) -> Option<ListDiff<SongModel>> {
        let songs = self.songs()?;
        let songs = songs.iter().enumerate().map(|(i, s)| s.to_song_model(i));

        match event {
            AppEvent::BrowserEvent(BrowserEvent::SavedTracksAppended(i)) => {
                Some(ListDiff::Append(songs.skip(*i).collect()))
            }
            _ => None,
        }
    }

    fn autoscroll_to_playing(&self) -> bool {
        true
    }

    fn actions_for(&self, id: &str) -> Option<gio::ActionGroup> {
        let songs = self.songs()?;
        let song = songs.iter().find(|&song| song.id == id)?;
        let group = SimpleActionGroup::new();

        for view_artist in song.make_artist_actions(self.dispatcher.box_clone(), None) {
            group.add_action(&view_artist);
        }
        group.add_action(&song.make_album_action(self.dispatcher.box_clone(), None));
        group.add_action(&song.make_link_action(None));

        Some(group.upcast())
    }

    fn menu_for(&self, id: &str) -> Option<gio::MenuModel> {
        let songs = self.songs()?;
        let song = songs.iter().find(|&song| song.id == id)?;

        let menu = gio::Menu::new();
        menu.append(Some(&*labels::VIEW_ALBUM), Some("song.view_album"));
        for artist in song.artists.iter() {
            menu.append(
                Some(&format!(
                    "{} {}",
                    *labels::MORE_FROM,
                    glib::markup_escape_text(&artist.name)
                )),
                Some(&format!("song.view_artist_{}", artist.id)),
            );
        }

        menu.append(Some(&*labels::COPY_LINK), Some("song.copy_link"));

        Some(menu.upcast())
    }

    fn select_song(&self, id: &str) {
        let song = self
            .songs()
            .and_then(|songs| songs.iter().find(|&song| song.id == id).cloned());
        if let Some(song) = song {
            self.dispatcher
                .dispatch(SelectionAction::Select(vec![song]).into());
        }
    }

    fn deselect_song(&self, id: &str) {
        self.dispatcher
            .dispatch(SelectionAction::Deselect(vec![id.to_string()]).into());
    }

    fn enable_selection(&self) -> bool {
        self.dispatcher
            .dispatch(AppAction::ChangeSelectionMode(true));
        true
    }

    fn selection(&self) -> Option<Box<dyn Deref<Target = SelectionState> + '_>> {
        let selection = self
            .app_model
            .map_state_opt(|s| Some(&s.selection))
            .filter(|s| s.context == SelectionContext::Queue)?;
        Some(Box::new(selection))
    }
}

impl SelectionToolsModel for SavedTracksModel {
    fn dispatcher(&self) -> Box<dyn ActionDispatcher> {
        self.dispatcher.box_clone()
    }

    fn spotify_client(&self) -> Arc<dyn SpotifyApiClient + Send + Sync> {
        self.app_model.get_spotify()
    }

    fn selection(&self) -> Option<Box<dyn Deref<Target = SelectionState> + '_>> {
        let selection = self
            .app_model
            .map_state_opt(|s| Some(&s.selection))
            .filter(|s| s.context == SelectionContext::Queue)?;
        Some(Box::new(selection))
    }

    fn tools_visible(&self, _: &SelectionState) -> Vec<SelectionTool> {
        vec![SelectionTool::Simple(SimpleSelectionTool::SelectAll)]
    }

    fn handle_tool_activated(&self, selection: &SelectionState, tool: &SelectionTool) {
        match tool {
            SelectionTool::Simple(SimpleSelectionTool::SelectAll) => {
                if let Some(songs) = self.songs() {
                    self.handle_select_all_tool(selection, &songs[..]);
                }
            }
            _ => self.default_handle_tool_activated(selection, tool),
        };
    }
}
