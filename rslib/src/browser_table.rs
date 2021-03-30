// Copyright: Ankitects Pty Ltd and contributors
// License: GNU AGPL, version 3 or later; http://www.gnu.org/licenses/agpl.html

use std::sync::Arc;

use itertools::Itertools;
use serde_repr::{Deserialize_repr, Serialize_repr};

use crate::err::{AnkiError, Result};
use crate::i18n::I18n;
use crate::{
    card::{Card, CardId, CardQueue, CardType},
    collection::Collection,
    config::BoolKey,
    decks::{Deck, DeckId},
    notes::{Note, NoteId},
    notetype::{CardTemplate, Notetype, NotetypeKind},
    scheduler::{timespan::time_span, timing::SchedTimingToday},
    template::RenderedNode,
    text::{extract_av_tags, html_to_text_line},
    timestamp::{TimestampMillis, TimestampSecs},
};

#[derive(Serialize_repr, Deserialize_repr, Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum Column {
    Custom = 0,
    Question = 1,
    Answer = 2,
    CardDeck = 3,
    CardDue = 4,
    CardEase = 5,
    CardLapses = 6,
    CardInterval = 7,
    CardMod = 8,
    CardReps = 9,
    CardTemplate = 10,
    NoteCards = 11,
    NoteCreation = 12,
    NoteEase = 13,
    NoteField = 14,
    NoteLapses = 15,
    NoteMod = 16,
    NoteReps = 17,
    NoteTags = 18,
    Notetype = 19,
}

#[derive(Debug, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub color: Color,
    pub font: Font,
}

#[derive(Debug, PartialEq)]
pub struct Cell {
    pub text: String,
    pub is_rtl: bool,
}

#[derive(Debug, PartialEq)]
pub enum Color {
    Default,
    Marked,
    Suspended,
    FlagRed,
    FlagOrange,
    FlagGreen,
    FlagBlue,
}

#[derive(Debug, PartialEq)]
pub struct Font {
    pub name: String,
    pub size: u32,
}

trait RowContext {
    fn get_cell_text(&mut self, column: &Column) -> Result<String>;
    fn get_row_color(&self) -> Color;
    fn get_row_font(&self) -> Result<Font>;
    fn note(&self) -> &Note;
    fn notetype(&self) -> &Notetype;

    fn get_cell(&mut self, column: &Column) -> Result<Cell> {
        Ok(Cell {
            text: self.get_cell_text(column)?,
            is_rtl: self.get_is_rtl(column),
        })
    }

    fn note_creation_str(&self) -> String {
        TimestampMillis(self.note().id.into())
            .as_secs()
            .date_string()
    }

    fn note_field_str(&self) -> String {
        let index = self.notetype().config.sort_field_idx as usize;
        html_to_text_line(&self.note().fields()[index]).into()
    }

    fn get_is_rtl(&self, column: &Column) -> bool {
        match column {
            Column::NoteField => {
                let index = self.notetype().config.sort_field_idx as usize;
                self.notetype().fields[index].config.rtl
            }
            _ => false,
        }
    }

    fn browser_row_for_id(&mut self, columns: &[Column]) -> Result<Row> {
        Ok(Row {
            cells: columns
                .iter()
                .map(|column| self.get_cell(column))
                .collect::<Result<_>>()?,
            color: self.get_row_color(),
            font: self.get_row_font()?,
        })
    }
}

struct CardRowContext<'a> {
    col: &'a Collection,
    card: Card,
    note: Note,
    notetype: Arc<Notetype>,
    deck: Option<Deck>,
    original_deck: Option<Option<Deck>>,
    tr: &'a I18n,
    timing: SchedTimingToday,
    render_context: Option<RenderContext>,
}

/// The answer string needs the question string but not the other way around, so only build the
/// answer string when needed.
struct RenderContext {
    question: String,
    answer_nodes: Vec<RenderedNode>,
}

struct NoteRowContext<'a> {
    note: Note,
    notetype: Arc<Notetype>,
    cards: Vec<Card>,
    tr: &'a I18n,
}

fn card_render_required(columns: &[Column]) -> bool {
    columns
        .iter()
        .any(|c| matches!(c, Column::Question | Column::Answer))
}

impl Card {
    fn is_new_type_or_queue(&self) -> bool {
        self.queue == CardQueue::New || self.ctype == CardType::New
    }

    fn is_filtered_deck(&self) -> bool {
        self.original_deck_id != DeckId(0)
    }

    /// Returns true if the card can not be due as it's buried or suspended.
    fn is_undue_queue(&self) -> bool {
        (self.queue as i8) < 0
    }

    /// Returns true of the card has a due date in terms of days.
    fn is_due_in_days(&self) -> bool {
        matches!(self.queue, CardQueue::DayLearn | CardQueue::Review)
            || (self.ctype == CardType::Review && self.is_undue_queue())
    }

    /// Returns the card's due date as a timestamp if it has one.
    fn due_time(&self, timing: &SchedTimingToday) -> Option<TimestampSecs> {
        if self.queue == CardQueue::Learn {
            Some(TimestampSecs(self.due as i64))
        } else if self.is_due_in_days() {
            Some(
                TimestampSecs::now()
                    .adding_secs(((self.due - timing.days_elapsed as i32) * 86400) as i64),
            )
        } else {
            None
        }
    }
}

impl Collection {
    pub fn browser_row_for_id(&mut self, id: i64) -> Result<Row> {
        if self.get_bool(BoolKey::BrowserTableShowNotesMode) {
            let columns =
                self.get_desktop_browser_note_columns()
                    .ok_or(AnkiError::InvalidInput {
                        info: "Note columns not set.".into(),
                    })?;
            NoteRowContext::new(self, id)?.browser_row_for_id(&columns)
        } else {
            let columns =
                self.get_desktop_browser_card_columns()
                    .ok_or(AnkiError::InvalidInput {
                        info: "Card columns not set.".into(),
                    })?;
            CardRowContext::new(self, id, card_render_required(&columns))?
                .browser_row_for_id(&columns)
        }
    }

    fn get_note_maybe_with_fields(&self, id: NoteId, _with_fields: bool) -> Result<Note> {
        // todo: After note.sort_field has been modified so it can be displayed in the browser,
        // we can update note_field_str() and only load the note with fields if a card render is
        // necessary (see #1082).
        if true {
            self.storage.get_note(id)?
        } else {
            self.storage.get_note_without_fields(id)?
        }
        .ok_or(AnkiError::NotFound)
    }
}

impl RenderContext {
    fn new(col: &mut Collection, card: &Card, note: &Note, notetype: &Notetype) -> Result<Self> {
        let render = col.render_card(
            note,
            card,
            notetype,
            notetype.get_template(card.template_idx)?,
            true,
        )?;
        let qnodes_text = render
            .qnodes
            .iter()
            .map(|node| match node {
                RenderedNode::Text { text } => text,
                RenderedNode::Replacement {
                    field_name: _,
                    current_text,
                    filters: _,
                } => current_text,
            })
            .join("");
        let question = extract_av_tags(&qnodes_text, true).0.to_string();

        Ok(RenderContext {
            question,
            answer_nodes: render.anodes,
        })
    }
}

impl<'a> CardRowContext<'a> {
    fn new(col: &'a mut Collection, id: i64, with_card_render: bool) -> Result<Self> {
        let card = col
            .storage
            .get_card(CardId(id))?
            .ok_or(AnkiError::NotFound)?;
        let note = col.get_note_maybe_with_fields(card.note_id, with_card_render)?;
        let notetype = col
            .get_notetype(note.notetype_id)?
            .ok_or(AnkiError::NotFound)?;
        let timing = col.timing_today()?;
        let render_context = if with_card_render {
            Some(RenderContext::new(col, &card, &note, &notetype)?)
        } else {
            None
        };

        Ok(CardRowContext {
            col,
            card,
            note,
            notetype,
            deck: None,
            original_deck: None,
            tr: &col.tr,
            timing,
            render_context,
        })
    }

    fn template(&self) -> Result<&CardTemplate> {
        self.notetype.get_template(self.card.template_idx)
    }

    fn deck(&mut self) -> Result<&Deck> {
        if self.deck.is_none() {
            self.deck = Some(
                self.col
                    .storage
                    .get_deck(self.card.deck_id)?
                    .ok_or(AnkiError::NotFound)?,
            );
        }
        Ok(self.deck.as_ref().unwrap())
    }

    fn original_deck(&mut self) -> Result<&Option<Deck>> {
        if self.original_deck.is_none() {
            self.original_deck = Some(self.col.storage.get_deck(self.card.original_deck_id)?);
        }
        Ok(self.original_deck.as_ref().unwrap())
    }

    fn answer_str(&self) -> String {
        let render_context = self.render_context.as_ref().unwrap();
        let answer = render_context
            .answer_nodes
            .iter()
            .map(|node| match node {
                RenderedNode::Text { text } => text,
                RenderedNode::Replacement {
                    field_name: _,
                    current_text,
                    filters: _,
                } => current_text,
            })
            .join("");
        let answer = extract_av_tags(&answer, false).0;
        html_to_text_line(
            if let Some(stripped) = answer.strip_prefix(&render_context.question) {
                stripped
            } else {
                &answer
            },
        )
        .to_string()
    }

    fn card_due_str(&mut self) -> String {
        let due = if self.card.is_filtered_deck() {
            self.tr.browsing_filtered()
        } else if self.card.is_new_type_or_queue() {
            self.tr.statistics_due_for_new_card(self.card.due)
        } else if let Some(time) = self.card.due_time(&self.timing) {
            time.date_string().into()
        } else {
            return "".into();
        };
        if self.card.is_undue_queue() {
            format!("({})", due)
        } else {
            due.into()
        }
    }

    fn card_ease_str(&self) -> String {
        match self.card.ctype {
            CardType::New => self.tr.browsing_new().into(),
            _ => format!("{}%", self.card.ease_factor / 10),
        }
    }

    fn card_interval_str(&self) -> String {
        match self.card.ctype {
            CardType::New => self.tr.browsing_new().into(),
            CardType::Learn => self.tr.browsing_learning().into(),
            _ => time_span((self.card.interval * 86400) as f32, self.tr, false),
        }
    }

    fn deck_str(&mut self) -> Result<String> {
        let deck_name = self.deck()?.human_name();
        Ok(if let Some(original_deck) = self.original_deck()? {
            format!("{} ({})", &deck_name, &original_deck.human_name())
        } else {
            deck_name
        })
    }

    fn template_str(&self) -> Result<String> {
        let name = &self.template()?.name;
        Ok(match self.notetype.config.kind() {
            NotetypeKind::Normal => name.to_owned(),
            NotetypeKind::Cloze => format!("{} {}", name, self.card.template_idx + 1),
        })
    }

    fn question_str(&self) -> String {
        html_to_text_line(&self.render_context.as_ref().unwrap().question).to_string()
    }
}

impl RowContext for CardRowContext<'_> {
    fn get_cell_text(&mut self, column: &Column) -> Result<String> {
        Ok(match column {
            Column::Question => self.question_str(),
            Column::Answer => self.answer_str(),
            Column::CardDeck => self.deck_str()?,
            Column::CardDue => self.card_due_str(),
            Column::CardEase => self.card_ease_str(),
            Column::CardInterval => self.card_interval_str(),
            Column::CardLapses => self.card.lapses.to_string(),
            Column::CardMod => self.card.mtime.date_string(),
            Column::CardReps => self.card.reps.to_string(),
            Column::CardTemplate => self.template_str()?,
            Column::NoteCreation => self.note_creation_str(),
            Column::NoteField => self.note_field_str(),
            Column::NoteMod => self.note.mtime.date_string(),
            Column::NoteTags => self.note.tags.join(" "),
            Column::Notetype => self.notetype.name.to_owned(),
            _ => "".to_string(),
        })
    }

    fn get_row_color(&self) -> Color {
        match self.card.flags {
            1 => Color::FlagRed,
            2 => Color::FlagOrange,
            3 => Color::FlagGreen,
            4 => Color::FlagBlue,
            _ => {
                if self
                    .note
                    .tags
                    .iter()
                    .any(|tag| tag.eq_ignore_ascii_case("marked"))
                {
                    Color::Marked
                } else if self.card.queue == CardQueue::Suspended {
                    Color::Suspended
                } else {
                    Color::Default
                }
            }
        }
    }

    fn get_row_font(&self) -> Result<Font> {
        Ok(Font {
            name: self.template()?.config.browser_font_name.to_owned(),
            size: self.template()?.config.browser_font_size,
        })
    }

    fn note(&self) -> &Note {
        &self.note
    }

    fn notetype(&self) -> &Notetype {
        &self.notetype
    }
}

impl<'a> NoteRowContext<'a> {
    fn new(col: &'a mut Collection, id: i64) -> Result<Self> {
        let note = col.get_note_maybe_with_fields(NoteId(id), false)?;
        let notetype = col
            .get_notetype(note.notetype_id)?
            .ok_or(AnkiError::NotFound)?;
        let cards = col.storage.all_cards_of_note(note.id)?;

        Ok(NoteRowContext {
            note,
            notetype,
            cards,
            tr: &col.tr,
        })
    }

    fn note_ease_str(&self) -> String {
        let cards = self
            .cards
            .iter()
            .filter(|c| c.ctype != CardType::New)
            .collect::<Vec<&Card>>();
        if cards.is_empty() {
            self.tr.browsing_new().into()
        } else {
            let ease = cards.iter().map(|c| c.ease_factor).sum::<u16>() / cards.len() as u16;
            format!("{}%", ease / 10)
        }
    }
}

impl RowContext for NoteRowContext<'_> {
    fn get_cell_text(&mut self, column: &Column) -> Result<String> {
        Ok(match column {
            Column::NoteCards => self.cards.len().to_string(),
            Column::NoteCreation => self.note_creation_str(),
            Column::NoteEase => self.note_ease_str(),
            Column::NoteField => self.note_field_str(),
            Column::NoteLapses => self.cards.iter().map(|c| c.lapses).sum::<u32>().to_string(),
            Column::NoteMod => self.note.mtime.date_string(),
            Column::NoteReps => self.cards.iter().map(|c| c.reps).sum::<u32>().to_string(),
            Column::NoteTags => self.note.tags.join(" "),
            Column::Notetype => self.notetype.name.to_owned(),
            _ => "".to_string(),
        })
    }

    fn get_row_color(&self) -> Color {
        if self
            .note
            .tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case("marked"))
        {
            Color::Marked
        } else {
            Color::Default
        }
    }

    fn get_row_font(&self) -> Result<Font> {
        Ok(Font {
            name: "".to_owned(),
            size: 0,
        })
    }

    fn note(&self) -> &Note {
        &self.note
    }

    fn notetype(&self) -> &Notetype {
        &self.notetype
    }
}
