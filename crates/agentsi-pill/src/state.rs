//! The states of the pill and the rules that move between them.
//!
//! This file holds no window code, so the tests run with no display.

use std::time::Duration;

/// How long the pill shows an error line.
pub const ERROR_SHOWN: Duration = Duration::from_secs(3);
/// How long the pill waits for the first sound after `voice.speak`.
pub const PREPARING_LIMIT: Duration = Duration::from_secs(10);
/// How long the pill waits for the text after a recording ends.
pub const TRANSCRIBING_LIMIT: Duration = Duration::from_secs(30);
/// The longest recording of hold dictation. At this limit the pill ends the recording and
/// transcribes it. Toggle dictation has no such limit: it sends a piece at each pause.
pub const LISTENING_LIMIT: Duration = Duration::from_secs(120);
/// How long toggle dictation listens with no speech before it stops.
pub const QUIET_LIMIT: Duration = Duration::from_secs(10 * 60);
/// The line when toggle dictation stops for lack of speech.
pub const QUIET_STOPPED: &str = "Dictation stopped: no speech for 10 min";
/// How long conversation mode listens with no turn before it stops.
pub const TURN_QUIET_LIMIT: Duration = QUIET_LIMIT;
/// The line when conversation mode stops for lack of a turn.
pub const TURN_QUIET_STOPPED: &str = "Conversation stopped: no turn for 10 min";
/// How long the pill waits for the reply of the pane. The Stop hook of the pane ends the wait.
/// A pane that the owner interrupts sends no Stop hook, so the wait has an end.
pub const WAITING_LIMIT: Duration = Duration::from_secs(30 * 60);
/// The line when conversation mode stops because the pane gave no reply.
pub const WAITING_STOPPED: &str = "Conversation stopped: no reply from the pane";
/// The line when a start of conversation mode finds no pane.
pub const NO_PANE: &str = "Focus a pane first";
/// The pause that ends a turn, in seconds.
pub const TURN_PAUSE: f32 = 1.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// No window exists.
    Hidden,
    /// A reply waits for its first sound.
    Preparing,
    Speaking,
    /// The microphone records.
    Listening,
    /// The recording ended and the text is not ready.
    Transcribing,
    /// Conversation mode sent a turn and the pane works. The microphone is closed.
    Waiting,
    /// One line for `ERROR_SHOWN`.
    Error,
}

impl Phase {
    pub const ALL: [Phase; 7] = [
        Phase::Hidden,
        Phase::Preparing,
        Phase::Speaking,
        Phase::Listening,
        Phase::Transcribing,
        Phase::Waiting,
        Phase::Error,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Phase::Hidden => "hidden",
            Phase::Preparing => "preparing",
            Phase::Speaking => "speaking",
            Phase::Listening => "listening",
            Phase::Transcribing => "transcribing",
            Phase::Waiting => "waiting",
            Phase::Error => "error",
        }
    }

    pub fn parse(name: &str) -> Option<Phase> {
        Phase::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The phases in which a speech runs.
    pub fn is_speech(self) -> bool {
        matches!(self, Phase::Preparing | Phase::Speaking)
    }

    /// A phase that draws a new frame on a timer. The others draw once per change. Waiting
    /// draws once: the pill waits for the pane with no frame.
    pub fn animated(self) -> bool {
        matches!(
            self,
            Phase::Preparing | Phase::Speaking | Phase::Listening | Phase::Transcribing
        )
    }

    /// How long the pill stays in this phase when nothing else happens.
    pub fn limit(self) -> Option<Duration> {
        match self {
            Phase::Preparing => Some(PREPARING_LIMIT),
            Phase::Listening => Some(LISTENING_LIMIT),
            Phase::Transcribing => Some(TRANSCRIBING_LIMIT),
            Phase::Waiting => Some(WAITING_LIMIT),
            Phase::Error => Some(ERROR_SHOWN),
            _ => None,
        }
    }
}

/// What the speech code reports about one speech.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechEvent {
    /// The first sound is in the output stream.
    Playing,
    /// The last sound played.
    Done,
    Failed(String),
}

/// What `converse.state` of the core tells the pill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Converse {
    Started { terminal: String },
    Stopped,
    SilentReply { terminal: String },
    NoPane,
}

/// What the speech-in code reports about one turn of conversation mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Turn {
    /// The turn ended and the text is not ready.
    Heard,
    /// The core submitted the text to the pane.
    Sent,
    /// The turn gave no text, or the core sent none.
    Nothing,
    /// The turn is the word "stop" or "para" alone.
    Stop,
    Failed(String),
}

/// The mark that tells the owner the microphone is open without a held key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Toggle dictation.
    Toggle,
    /// Conversation mode.
    Conversation,
}

/// What reaches the pill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// `voice.speak` from the core.
    Speak {
        terminal: String,
        role: String,
        text: String,
    },
    /// `voice.stop` from the core.
    CoreStop { terminal: String },
    /// The speech or the speech-in code sets the phase.
    Set(Phase),
    /// The speech or the speech-in code reports a failure.
    Fail(String),
    /// The speech code reports on the speech of `ticket`. An old ticket does nothing.
    Speech { ticket: u64, event: SpeechEvent },
    /// `dictation.start` from the core: the owner holds the key, or pressed it in toggle form.
    /// `at_us` is when. `toggle` is true when the key file sets toggle dictation.
    DictationStart { at_us: u64, toggle: bool },
    /// `dictation.stop` from the core: the owner released the key. `at_us` is when.
    DictationStop { at_us: u64 },
    /// The speech-in code ends the dictation of `ticket`: the text reached the core, or it
    /// failed. The result of an old dictation does nothing.
    Dictated {
        ticket: u64,
        result: Result<(), String>,
    },
    /// The speech-in code heard no speech in toggle dictation `ticket` for [`QUIET_LIMIT`].
    Quiet { ticket: u64 },
    /// `converse.state` from the core.
    Converse(Converse),
    /// The speech-in code reports on the turn of listening `ticket`. An old ticket does nothing.
    Turn { ticket: u64, turn: Turn },
    /// A click on the pill.
    Click,
    /// The limit of a phase ended. `generation` tells which phase started it.
    Timeout { generation: u64 },
    /// The link to the core broke.
    Disconnected,
}

/// What the caller must do after an input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// `ticket` grows by one at each start, so the speech code can name the speech it reports on.
    StartSpeech {
        ticket: u64,
        terminal: String,
        role: String,
        text: String,
    },
    StopSpeech {
        terminal: Option<String>,
    },
    /// Ask the core to end the recording, as a click does. The core answers with `dictation.stop`.
    StopRecording,
    /// Open the microphone for the dictation `ticket`. With `toggle`, the speech-in code cuts
    /// the speech at the pauses and sends each piece at once.
    StartRecording {
        ticket: u64,
        toggle: bool,
    },
    /// Close the microphone, then transcribe what it heard.
    FinishRecording {
        ticket: u64,
    },
    /// Close the microphone and drop what it heard.
    CancelRecording,
    /// Open the microphone for one turn of conversation mode `ticket`. The speech-in code
    /// closes it when the turn ends and reports with `Input::Turn`.
    Listen {
        ticket: u64,
    },
    /// Ask the core to end conversation mode, as a click does. The core answers with
    /// `converse.state` `stopped`.
    EndConversation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Conversation {
    terminal: String,
    ticket: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    phase: Phase,
    /// The terminal of the speech in progress.
    terminal: Option<String>,
    /// The line that the error phase shows.
    error: Option<String>,
    /// Grows at each change of phase, so a late timer does nothing.
    generation: u64,
    /// The ticket of the last `StartSpeech`.
    ticket: u64,
    /// Grows at each dictation that starts, so the result of an old one does nothing.
    dictation: u64,
    /// When the last `dictation.stop` happened. A `dictation.start` from before it is stale.
    last_stop_us: u64,
    /// The dictation in progress is toggle dictation.
    toggle: bool,
    /// Conversation mode: the pane, and the ticket of the turn that the microphone listens to.
    conversation: Option<Conversation>,
    /// The ticket that started the speech phases now shown. The replies from it to
    /// `ticket` play one after the other, so the first sound of any of them counts.
    first_ticket: u64,
}

impl Default for Model {
    fn default() -> Model {
        Model {
            phase: Phase::Hidden,
            terminal: None,
            error: None,
            generation: 0,
            ticket: 0,
            dictation: 0,
            last_stop_us: 0,
            toggle: false,
            conversation: None,
            first_ticket: 0,
        }
    }
}

impl Model {
    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn terminal(&self) -> Option<&str> {
        self.terminal.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn ticket(&self) -> u64 {
        self.ticket
    }

    /// The dictation in progress is toggle dictation: the microphone stays open until a second
    /// press or a click.
    pub fn toggle(&self) -> bool {
        self.toggle
    }

    /// The pane of conversation mode, while it runs.
    pub fn conversation(&self) -> Option<&str> {
        self.conversation.as_ref().map(|c| c.terminal.as_str())
    }

    /// The mark for the phase now shown: conversation mode in each phase of it, toggle
    /// dictation while it listens.
    pub fn mark(&self) -> Option<Mark> {
        if self.conversation.is_some() && self.phase != Phase::Error {
            Some(Mark::Conversation)
        } else if self.toggle && self.phase == Phase::Listening {
            Some(Mark::Toggle)
        } else {
            None
        }
    }

    /// The same model, in conversation mode with `terminal`. For the screenshots.
    pub fn with_conversation(mut self, terminal: &str) -> Model {
        self.conversation = Some(Conversation {
            terminal: terminal.to_owned(),
            ticket: 0,
        });
        self
    }

    /// How long the pill stays in this phase when nothing else happens. Toggle dictation
    /// and conversation mode listen with no limit of the pill: the speech-in code reports
    /// `Quiet`.
    pub fn limit(&self) -> Option<Duration> {
        if self.phase == Phase::Listening && (self.toggle || self.conversation.is_some()) {
            None
        } else {
            self.phase.limit()
        }
    }

    /// The same model, in toggle dictation. For the screenshots.
    pub fn with_toggle(mut self) -> Model {
        self.toggle = true;
        self
    }

    /// A model that starts in `phase`, for the screenshots and the tests.
    pub fn in_phase(phase: Phase, error: Option<&str>) -> Model {
        let mut model = Model::default();
        model.enter(phase);
        model.error = error.map(str::to_owned);
        model
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.generation += 1;
        if !matches!(phase, Phase::Preparing | Phase::Speaking) {
            self.terminal = None;
        }
        if phase != Phase::Error {
            self.error = None;
        }
    }

    /// Leaves the phase and stops the speech that it ran, if any.
    fn stop_speech(&mut self, next: Phase) -> Vec<Effect> {
        let terminal = self.terminal.take();
        let was_speech = self.phase.is_speech();
        self.enter(next);
        if was_speech {
            vec![Effect::StopSpeech { terminal }]
        } else {
            Vec::new()
        }
    }

    fn fail(&mut self, message: impl Into<String>) -> Vec<Effect> {
        let was_listening = self.phase == Phase::Listening;
        let conversing = self.conversation.take().is_some();
        let mut effects = self.stop_speech(Phase::Error);
        if conversing {
            // The core ends the mode for every client. A second failure finds no mode to end.
            effects.push(Effect::CancelRecording);
            effects.push(Effect::EndConversation);
        } else if was_listening {
            effects.push(Effect::CancelRecording);
            // No key release follows toggle dictation. The core ends it, so that its next
            // press starts a dictation.
            if self.toggle {
                effects.push(Effect::StopRecording);
            }
        }
        self.error = Some(message.into());
        effects
    }

    /// The last sentence of a reply played. Conversation mode listens again.
    fn reply_done(&mut self) -> Vec<Effect> {
        if self.conversation.is_some() {
            self.listen()
        } else {
            self.enter(Phase::Hidden);
            Vec::new()
        }
    }

    /// Conversation mode opens the microphone for the next turn.
    fn listen(&mut self) -> Vec<Effect> {
        self.dictation += 1;
        let ticket = self.dictation;
        if let Some(c) = self.conversation.as_mut() {
            c.ticket = ticket;
        }
        self.enter(Phase::Listening);
        vec![Effect::Listen { ticket }]
    }

    fn converse(&mut self, event: Converse) -> Vec<Effect> {
        match event {
            Converse::Started { terminal } => {
                let was_listening = matches!(self.phase, Phase::Listening | Phase::Transcribing);
                let mut effects = self.stop_speech(Phase::Hidden);
                if was_listening {
                    effects.push(Effect::CancelRecording);
                }
                self.toggle = false;
                self.conversation = Some(Conversation {
                    terminal,
                    ticket: 0,
                });
                effects.extend(self.listen());
                effects
            }
            Converse::Stopped => {
                if self.conversation.take().is_none() {
                    return Vec::new();
                }
                let mut effects = self.stop_speech(Phase::Hidden);
                effects.push(Effect::CancelRecording);
                effects
            }
            Converse::SilentReply { terminal } => {
                let mine = self
                    .conversation
                    .as_ref()
                    .is_some_and(|c| c.terminal == terminal);
                if mine && self.phase == Phase::Waiting {
                    self.listen()
                } else {
                    Vec::new()
                }
            }
            Converse::NoPane => {
                // A mode or a dictation that runs is not the business of this start.
                if matches!(self.phase, Phase::Hidden | Phase::Error) && self.conversation.is_none()
                {
                    self.fail(NO_PANE)
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn turn(&mut self, ticket: u64, turn: Turn) -> Vec<Effect> {
        if self.conversation.as_ref().map(|c| c.ticket) != Some(ticket) {
            return Vec::new();
        }
        match (turn, self.phase) {
            (Turn::Heard, Phase::Listening) => {
                self.enter(Phase::Transcribing);
                Vec::new()
            }
            (Turn::Sent, Phase::Transcribing) => {
                self.enter(Phase::Waiting);
                Vec::new()
            }
            (Turn::Nothing, Phase::Transcribing | Phase::Listening) => self.listen(),
            // The mode ends by the answer of the core.
            (Turn::Stop, _) => vec![Effect::EndConversation],
            (Turn::Failed(message), _) => self.fail(message),
            _ => Vec::new(),
        }
    }

    pub fn apply(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Speak {
                terminal,
                role,
                text,
            } => {
                // The microphone wins. A reply that comes during a recording is not spoken.
                if matches!(self.phase, Phase::Listening | Phase::Transcribing) {
                    return Vec::new();
                }
                // Conversation mode hears the reply of its pane and no other.
                if self
                    .conversation
                    .as_ref()
                    .is_some_and(|c| c.terminal != terminal)
                {
                    return Vec::new();
                }
                let mut effects = Vec::new();
                let mut fresh = !self.phase.is_speech();
                if fresh {
                    self.enter(Phase::Preparing);
                } else if self.terminal.as_deref().is_some_and(|old| old != terminal) {
                    // Another terminal takes over. The speech code must drop the old one.
                    effects.push(Effect::StopSpeech {
                        terminal: self.terminal.clone(),
                    });
                    fresh = true;
                }
                self.terminal = Some(terminal.clone());
                self.ticket += 1;
                if fresh {
                    self.first_ticket = self.ticket;
                }
                effects.push(Effect::StartSpeech {
                    ticket: self.ticket,
                    terminal,
                    role,
                    text,
                });
                effects
            }
            Input::CoreStop { terminal } => {
                if self.phase.is_speech() && self.terminal.as_deref() == Some(terminal.as_str()) {
                    // In conversation mode, a new prompt in the pane means a new wait.
                    let next = if self.conversation.is_some() {
                        Phase::Waiting
                    } else {
                        Phase::Hidden
                    };
                    self.stop_speech(next)
                } else {
                    Vec::new()
                }
            }
            Input::Set(phase) => {
                if phase != self.phase {
                    self.enter(phase);
                }
                Vec::new()
            }
            Input::Fail(message) => self.fail(message),
            Input::Speech { ticket, event } => {
                let current = (self.first_ticket..=self.ticket).contains(&ticket);
                // Only the last reply ends the speech. The others play before it.
                let last = ticket == self.ticket;
                if !self.phase.is_speech() || !current || (event == SpeechEvent::Done && !last) {
                    return Vec::new();
                }
                match event {
                    SpeechEvent::Playing => {
                        if self.phase == Phase::Preparing {
                            self.enter(Phase::Speaking);
                        }
                        Vec::new()
                    }
                    SpeechEvent::Done => self.reply_done(),
                    SpeechEvent::Failed(message) => self.fail(message),
                }
            }
            Input::DictationStart { at_us, toggle } => match self.phase {
                Phase::Listening => Vec::new(),
                // The last piece of the old dictation is still in the model. The core counts
                // this press as a start, so it gets a stop to match.
                Phase::Transcribing if toggle => vec![Effect::StopRecording],
                Phase::Transcribing => Vec::new(),
                // The key was released before this press arrived: the two commands of a quick
                // tap are two processes, and the stop got here first.
                _ if at_us < self.last_stop_us => Vec::new(),
                _ => {
                    self.dictation += 1;
                    self.toggle = toggle;
                    let mut effects = self.stop_speech(Phase::Listening);
                    effects.push(Effect::StartRecording {
                        ticket: self.dictation,
                        toggle,
                    });
                    effects
                }
            },
            Input::DictationStop { at_us } => {
                self.last_stop_us = self.last_stop_us.max(at_us);
                if self.phase == Phase::Listening {
                    self.enter(Phase::Transcribing);
                    vec![Effect::FinishRecording {
                        ticket: self.dictation,
                    }]
                } else {
                    Vec::new()
                }
            }
            Input::Dictated { ticket, result } => {
                let live = matches!(self.phase, Phase::Listening | Phase::Transcribing);
                if ticket != self.dictation || !live {
                    return Vec::new();
                }
                match result {
                    Ok(()) if self.phase == Phase::Transcribing => {
                        self.enter(Phase::Hidden);
                        Vec::new()
                    }
                    Ok(()) => Vec::new(),
                    Err(message) => self.fail(message),
                }
            }
            Input::Quiet { ticket } => {
                if self.phase == Phase::Listening && ticket == self.dictation {
                    let line = if self.conversation.is_some() {
                        TURN_QUIET_STOPPED
                    } else {
                        QUIET_STOPPED
                    };
                    self.fail(line)
                } else {
                    Vec::new()
                }
            }
            Input::Converse(event) => self.converse(event),
            Input::Turn { ticket, turn } => self.turn(ticket, turn),
            Input::Click if self.conversation.is_some() && self.phase != Phase::Error => {
                // The core ends the mode for every client, and answers with `stopped`.
                vec![Effect::EndConversation]
            }
            Input::Click => match self.phase {
                Phase::Preparing | Phase::Speaking => self.stop_speech(Phase::Hidden),
                // The core ends the recording for every client, so the key and the click act alike.
                Phase::Listening => vec![Effect::StopRecording],
                Phase::Error => self.stop_speech(Phase::Hidden),
                Phase::Hidden | Phase::Transcribing | Phase::Waiting => Vec::new(),
            },
            Input::Timeout { generation } => {
                if generation != self.generation {
                    return Vec::new();
                }
                match self.phase {
                    Phase::Error => self.stop_speech(Phase::Hidden),
                    Phase::Preparing => self.fail("The speech did not start"),
                    Phase::Listening => {
                        self.enter(Phase::Transcribing);
                        vec![Effect::FinishRecording {
                            ticket: self.dictation,
                        }]
                    }
                    Phase::Transcribing => self.fail("The text did not arrive"),
                    Phase::Waiting => self.fail(WAITING_STOPPED),
                    _ => Vec::new(),
                }
            }
            Input::Disconnected => {
                if self.conversation.take().is_some() {
                    // The core that would end the mode is gone.
                    let mut effects = self.stop_speech(Phase::Hidden);
                    effects.push(Effect::CancelRecording);
                    effects
                } else if self.phase.is_speech() {
                    self.stop_speech(Phase::Hidden)
                } else if self.phase == Phase::Listening {
                    // The core that would send the stop is gone.
                    self.enter(Phase::Hidden);
                    vec![Effect::CancelRecording]
                } else {
                    Vec::new()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speak(terminal: &str) -> Input {
        Input::Speak {
            terminal: terminal.into(),
            role: "pm".into(),
            text: "Hello.".into(),
        }
    }

    #[test]
    fn a_new_model_is_hidden_and_does_not_animate() {
        let model = Model::default();
        assert_eq!(model.phase(), Phase::Hidden);
        assert!(!model.phase().animated());
        assert_eq!(model.phase().limit(), None);
    }

    #[test]
    fn a_speak_event_shows_the_preparing_phase_and_starts_the_speech() {
        let mut model = Model::default();
        let effects = model.apply(speak("t1"));
        assert_eq!(model.phase(), Phase::Preparing);
        assert_eq!(model.terminal(), Some("t1"));
        assert_eq!(
            effects,
            vec![Effect::StartSpeech {
                ticket: 1,
                terminal: "t1".into(),
                role: "pm".into(),
                text: "Hello.".into()
            }]
        );
    }

    #[test]
    fn a_second_speak_event_keeps_the_phase_and_the_generation() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        model.apply(Input::Set(Phase::Speaking));
        let generation = model.generation();
        let effects = model.apply(speak("t1"));
        assert_eq!(model.phase(), Phase::Speaking);
        assert_eq!(model.generation(), generation);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn a_speak_event_during_a_recording_does_nothing() {
        let mut model = Model::in_phase(Phase::Listening, None);
        assert!(model.apply(speak("t1")).is_empty());
        assert_eq!(model.phase(), Phase::Listening);
    }

    #[test]
    fn a_stop_event_of_the_same_terminal_hides_the_pill() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        let effects = model.apply(Input::CoreStop {
            terminal: "t1".into(),
        });
        assert_eq!(model.phase(), Phase::Hidden);
        assert_eq!(
            effects,
            vec![Effect::StopSpeech {
                terminal: Some("t1".into())
            }]
        );
    }

    #[test]
    fn a_stop_event_of_another_terminal_does_nothing() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        assert!(
            model
                .apply(Input::CoreStop {
                    terminal: "t2".into()
                })
                .is_empty()
        );
        assert_eq!(model.phase(), Phase::Preparing);
    }

    #[test]
    fn a_click_during_speech_stops_it() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        model.apply(Input::Set(Phase::Speaking));
        let effects = model.apply(Input::Click);
        assert_eq!(model.phase(), Phase::Hidden);
        assert_eq!(
            effects,
            vec![Effect::StopSpeech {
                terminal: Some("t1".into())
            }]
        );
    }

    #[test]
    fn a_click_during_a_recording_stops_it_and_waits_for_the_text() {
        let mut model = Model::in_phase(Phase::Listening, None);
        let effects = model.apply(Input::Click);
        assert_eq!(model.phase(), Phase::Listening);
        assert_eq!(effects, vec![Effect::StopRecording]);
        let effects = model.apply(Input::DictationStop { at_us: 1 });
        assert_eq!(model.phase(), Phase::Transcribing);
        assert_eq!(effects, vec![Effect::FinishRecording { ticket: 0 }]);
    }

    #[test]
    fn the_key_starts_a_recording_and_stops_a_speech() {
        let mut model = Model::default();
        assert_eq!(
            model.apply(Input::DictationStart {
                at_us: 1,
                toggle: false
            }),
            vec![Effect::StartRecording {
                ticket: 1,
                toggle: false
            }]
        );
        assert_eq!(model.phase(), Phase::Listening);
        // A second press (key repeat) changes nothing.
        assert!(
            model
                .apply(Input::DictationStart {
                    at_us: 2,
                    toggle: false
                })
                .is_empty()
        );

        let mut model = Model::default();
        model.apply(speak("t1"));
        let effects = model.apply(Input::DictationStart {
            at_us: 1,
            toggle: false,
        });
        assert_eq!(model.phase(), Phase::Listening);
        assert_eq!(
            effects,
            vec![
                Effect::StopSpeech {
                    terminal: Some("t1".into())
                },
                Effect::StartRecording {
                    ticket: 1,
                    toggle: false
                }
            ]
        );
    }

    #[test]
    fn a_release_ends_the_recording_and_the_text_hides_the_pill() {
        let mut model = Model::default();
        model.apply(Input::DictationStart {
            at_us: 1,
            toggle: false,
        });
        assert_eq!(
            model.apply(Input::DictationStop { at_us: 2 }),
            vec![Effect::FinishRecording { ticket: 1 }]
        );
        assert_eq!(model.phase(), Phase::Transcribing);
        assert!(
            model
                .apply(Input::DictationStart {
                    at_us: 3,
                    toggle: false
                })
                .is_empty()
        );
        model.apply(Input::Dictated {
            ticket: 1,
            result: Ok(()),
        });
        assert_eq!(model.phase(), Phase::Hidden);
        // A release with no recording does nothing.
        assert!(model.apply(Input::DictationStop { at_us: 4 }).is_empty());
    }

    #[test]
    fn a_stop_that_arrives_before_its_start_cancels_that_start() {
        // A quick tap: the bind runs `start` and `stop` as two processes, and the core gets
        // the stop first. The press is older than the release, so it must not record.
        let mut model = Model::default();
        assert!(model.apply(Input::DictationStop { at_us: 20 }).is_empty());
        assert!(
            model
                .apply(Input::DictationStart {
                    at_us: 10,
                    toggle: false
                })
                .is_empty()
        );
        assert_eq!(model.phase(), Phase::Hidden);
        // The next press is later than that release and records.
        assert_eq!(
            model.apply(Input::DictationStart {
                at_us: 30,
                toggle: false
            }),
            vec![Effect::StartRecording {
                ticket: 1,
                toggle: false
            }]
        );
        assert_eq!(model.phase(), Phase::Listening);
    }

    #[test]
    fn the_result_of_an_old_dictation_does_nothing() {
        let mut model = Model::default();
        model.apply(Input::DictationStart {
            at_us: 1,
            toggle: false,
        });
        model.apply(Input::DictationStop { at_us: 2 });
        // The text takes too long, and the pill gives up.
        let generation = model.generation();
        model.apply(Input::Timeout { generation });
        assert_eq!(model.phase(), Phase::Error);
        model.apply(Input::Click);
        // A new dictation starts while the old one still runs in the worker.
        assert_eq!(
            model.apply(Input::DictationStart {
                at_us: 3,
                toggle: false
            }),
            vec![Effect::StartRecording {
                ticket: 2,
                toggle: false
            }]
        );
        let late_fail = Input::Dictated {
            ticket: 1,
            result: Err("late".into()),
        };
        assert!(model.apply(late_fail).is_empty());
        assert_eq!(model.phase(), Phase::Listening);
        let late_text = Input::Dictated {
            ticket: 1,
            result: Ok(()),
        };
        assert!(model.apply(late_text).is_empty());
        assert_eq!(model.phase(), Phase::Listening);
        // The new dictation still ends as usual.
        model.apply(Input::DictationStop { at_us: 4 });
        model.apply(Input::Dictated {
            ticket: 2,
            result: Ok(()),
        });
        assert_eq!(model.phase(), Phase::Hidden);
    }

    #[test]
    fn a_late_failure_after_the_pill_gave_up_shows_nothing() {
        let mut model = Model::default();
        model.apply(Input::DictationStart {
            at_us: 1,
            toggle: false,
        });
        model.apply(Input::DictationStop { at_us: 2 });
        let generation = model.generation();
        model.apply(Input::Timeout { generation });
        model.apply(Input::Click);
        assert_eq!(model.phase(), Phase::Hidden);
        let late_fail = Input::Dictated {
            ticket: 1,
            result: Err("late".into()),
        };
        assert!(model.apply(late_fail).is_empty());
        assert_eq!(model.phase(), Phase::Hidden);
    }

    #[test]
    fn a_failure_while_listening_closes_the_microphone() {
        let mut model = Model::in_phase(Phase::Listening, None);
        let effects = model.apply(Input::Fail("No microphone".into()));
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(effects, vec![Effect::CancelRecording]);
    }

    #[test]
    fn a_recording_ends_by_itself_at_the_limit() {
        let mut model = Model::in_phase(Phase::Listening, None);
        assert_eq!(model.phase().limit(), Some(LISTENING_LIMIT));
        let generation = model.generation();
        let effects = model.apply(Input::Timeout { generation });
        assert_eq!(model.phase(), Phase::Transcribing);
        assert_eq!(effects, vec![Effect::FinishRecording { ticket: 0 }]);
    }

    fn toggle_start(at_us: u64) -> Input {
        Input::DictationStart {
            at_us,
            toggle: true,
        }
    }

    #[test]
    fn toggle_dictation_listens_with_no_limit_until_the_second_press_or_a_click() {
        let mut model = Model::default();
        assert_eq!(
            model.apply(toggle_start(1)),
            vec![Effect::StartRecording {
                ticket: 1,
                toggle: true
            }]
        );
        assert_eq!(model.phase(), Phase::Listening);
        assert!(model.toggle());
        // Hold dictation ends at 120 s. Toggle dictation does not.
        assert_eq!(model.limit(), None);
        assert_eq!(
            Model::in_phase(Phase::Listening, None).limit(),
            Some(LISTENING_LIMIT)
        );
        // The click asks the core, which answers with the stop, as in hold dictation.
        assert_eq!(model.apply(Input::Click), vec![Effect::StopRecording]);
        assert_eq!(
            model.apply(Input::DictationStop { at_us: 2 }),
            vec![Effect::FinishRecording { ticket: 1 }]
        );
        assert_eq!(model.phase(), Phase::Transcribing);
        assert_eq!(model.limit(), Some(TRANSCRIBING_LIMIT));
    }

    #[test]
    fn no_speech_for_10_minutes_stops_toggle_dictation_with_a_line_and_tells_the_core() {
        let mut model = Model::default();
        model.apply(toggle_start(1));
        // A report on an old dictation does nothing.
        assert!(model.apply(Input::Quiet { ticket: 9 }).is_empty());
        assert_eq!(model.phase(), Phase::Listening);
        let effects = model.apply(Input::Quiet { ticket: 1 });
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(
            model.error(),
            Some("Dictation stopped: no speech for 10 min")
        );
        assert_eq!(
            effects,
            vec![Effect::CancelRecording, Effect::StopRecording]
        );
        assert_eq!(QUIET_LIMIT, Duration::from_secs(600));
        // After that, nothing is left to report.
        assert!(model.apply(Input::Quiet { ticket: 1 }).is_empty());
    }

    #[test]
    fn a_failure_of_toggle_dictation_tells_the_core_so_the_next_press_starts_again() {
        let mut model = Model::default();
        model.apply(toggle_start(1));
        let effects = model.apply(Input::Dictated {
            ticket: 1,
            result: Err("No microphone".into()),
        });
        assert_eq!(
            effects,
            vec![Effect::CancelRecording, Effect::StopRecording]
        );
        // Hold dictation has the release of the key for that.
        let mut model = Model::default();
        model.apply(Input::DictationStart {
            at_us: 1,
            toggle: false,
        });
        assert_eq!(
            model.apply(Input::Fail("x".into())),
            vec![Effect::CancelRecording]
        );
    }

    #[test]
    fn a_press_of_toggle_dictation_while_the_last_piece_is_transcribed_gets_a_stop_for_the_core() {
        let mut model = Model::default();
        model.apply(toggle_start(1));
        model.apply(Input::DictationStop { at_us: 2 });
        assert_eq!(model.phase(), Phase::Transcribing);
        assert_eq!(model.apply(toggle_start(3)), vec![Effect::StopRecording]);
        assert_eq!(model.phase(), Phase::Transcribing);
        // The next press starts a new dictation.
        model.apply(Input::Dictated {
            ticket: 1,
            result: Ok(()),
        });
        assert_eq!(
            model.apply(toggle_start(5)),
            vec![Effect::StartRecording {
                ticket: 2,
                toggle: true
            }]
        );
    }

    #[test]
    fn a_click_while_transcribing_does_nothing() {
        let mut model = Model::in_phase(Phase::Transcribing, None);
        assert!(model.apply(Input::Click).is_empty());
        assert_eq!(model.phase(), Phase::Transcribing);
    }

    #[test]
    fn a_click_on_an_error_hides_it() {
        let mut model = Model::default();
        model.apply(Input::Fail("No microphone".into()));
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(model.error(), Some("No microphone"));
        model.apply(Input::Click);
        assert_eq!(model.phase(), Phase::Hidden);
        assert_eq!(model.error(), None);
    }

    #[test]
    fn the_error_line_leaves_after_3_seconds() {
        let mut model = Model::default();
        model.apply(Input::Fail("x".into()));
        assert_eq!(model.phase().limit(), Some(Duration::from_secs(3)));
        let generation = model.generation();
        model.apply(Input::Timeout { generation });
        assert_eq!(model.phase(), Phase::Hidden);
    }

    #[test]
    fn a_late_timer_does_nothing() {
        let mut model = Model::default();
        model.apply(Input::Fail("first".into()));
        let old = model.generation();
        model.apply(Input::Fail("second".into()));
        model.apply(Input::Timeout { generation: old });
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(model.error(), Some("second"));
    }

    #[test]
    fn a_speech_that_never_starts_ends_in_an_error() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        let generation = model.generation();
        let effects = model.apply(Input::Timeout { generation });
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(
            effects,
            vec![Effect::StopSpeech {
                terminal: Some("t1".into())
            }]
        );
    }

    #[test]
    fn a_text_that_never_arrives_ends_in_an_error() {
        let mut model = Model::in_phase(Phase::Transcribing, None);
        let generation = model.generation();
        model.apply(Input::Timeout { generation });
        assert_eq!(model.phase(), Phase::Error);
    }

    #[test]
    fn a_broken_link_stops_the_speech_and_the_recording() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        let effects = model.apply(Input::Disconnected);
        assert_eq!(model.phase(), Phase::Hidden);
        assert_eq!(effects.len(), 1);
        let mut model = Model::in_phase(Phase::Listening, None);
        assert_eq!(
            model.apply(Input::Disconnected),
            vec![Effect::CancelRecording]
        );
        assert_eq!(model.phase(), Phase::Hidden);
    }

    #[test]
    fn a_failure_during_a_speech_stops_the_speech() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        let effects = model.apply(Input::Fail("late".into()));
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(
            effects,
            vec![Effect::StopSpeech {
                terminal: Some("t1".into())
            }]
        );
        assert_eq!(model.terminal(), None);
    }

    #[test]
    fn a_speech_of_another_terminal_stops_the_first() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        let effects = model.apply(speak("t2"));
        assert_eq!(model.terminal(), Some("t2"));
        assert_eq!(effects.len(), 2);
        assert_eq!(
            effects[0],
            Effect::StopSpeech {
                terminal: Some("t1".into())
            }
        );
        assert!(matches!(effects[1], Effect::StartSpeech { .. }));
        assert!(
            model
                .apply(Input::CoreStop {
                    terminal: "t1".into()
                })
                .is_empty()
        );
    }

    fn speech(ticket: u64, event: SpeechEvent) -> Input {
        Input::Speech { ticket, event }
    }

    #[test]
    fn the_speech_code_moves_a_speech_to_speaking_and_then_hidden() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        assert_eq!(model.ticket(), 1);
        assert!(model.apply(speech(1, SpeechEvent::Playing)).is_empty());
        assert_eq!(model.phase(), Phase::Speaking);
        model.apply(speech(1, SpeechEvent::Playing));
        assert_eq!(model.phase(), Phase::Speaking);
        assert!(model.apply(speech(1, SpeechEvent::Done)).is_empty());
        assert_eq!(model.phase(), Phase::Hidden);
        assert_eq!(model.terminal(), None);
    }

    #[test]
    fn a_report_on_an_old_speech_does_nothing() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        model.apply(speak("t1"));
        assert_eq!(model.ticket(), 2);
        model.apply(speech(1, SpeechEvent::Done));
        assert_eq!(model.phase(), Phase::Preparing);
        // After a stop, a late report of the stopped speech does not show the pill again.
        model.apply(Input::CoreStop {
            terminal: "t1".into(),
        });
        model.apply(speech(2, SpeechEvent::Playing));
        assert_eq!(model.phase(), Phase::Hidden);
    }

    #[test]
    fn the_first_sound_of_an_earlier_reply_shows_speaking() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        model.apply(speak("t1"));
        model.apply(speech(1, SpeechEvent::Playing));
        assert_eq!(
            model.phase(),
            Phase::Speaking,
            "the 10 s limit no longer runs"
        );
        assert_eq!(model.phase().limit(), None);
        model.apply(speech(1, SpeechEvent::Done));
        assert_eq!(model.phase(), Phase::Speaking);
        model.apply(speech(2, SpeechEvent::Done));
        assert_eq!(model.phase(), Phase::Hidden);
    }

    #[test]
    fn a_reply_of_a_terminal_that_lost_the_pill_does_not_count() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        model.apply(speak("t2"));
        model.apply(speech(1, SpeechEvent::Playing));
        assert_eq!(model.phase(), Phase::Preparing);
        model.apply(speech(2, SpeechEvent::Playing));
        assert_eq!(model.phase(), Phase::Speaking);
    }

    #[test]
    fn a_failed_speech_shows_its_error() {
        let mut model = Model::default();
        model.apply(speak("t1"));
        let effects = model.apply(speech(
            1,
            SpeechEvent::Failed("espeak-ng is not installed".into()),
        ));
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(model.error(), Some("espeak-ng is not installed"));
        assert_eq!(
            effects,
            vec![Effect::StopSpeech {
                terminal: Some("t1".into())
            }]
        );
    }

    fn started(terminal: &str) -> Input {
        Input::Converse(Converse::Started {
            terminal: terminal.into(),
        })
    }

    fn turn(ticket: u64, turn: Turn) -> Input {
        Input::Turn { ticket, turn }
    }

    /// A conversation with the pane `t1`, in the phase Listening for turn 1.
    fn conversing() -> Model {
        let mut model = Model::default();
        assert_eq!(
            model.apply(started("t1")),
            vec![Effect::Listen { ticket: 1 }]
        );
        model
    }

    /// A conversation whose first turn is sent: it waits for the pane.
    fn waiting() -> Model {
        let mut model = conversing();
        model.apply(turn(1, Turn::Heard));
        model.apply(turn(1, Turn::Sent));
        assert_eq!(model.phase(), Phase::Waiting);
        model
    }

    #[test]
    fn conversation_mode_listens_at_the_start_and_shows_its_mark() {
        let mut model = Model::default();
        assert_eq!(model.mark(), None);
        let effects = model.apply(started("t1"));
        assert_eq!(effects, vec![Effect::Listen { ticket: 1 }]);
        assert_eq!(model.phase(), Phase::Listening);
        assert_eq!(model.conversation(), Some("t1"));
        assert_eq!(model.mark(), Some(Mark::Conversation));
        // The microphone stays open with no limit of the pill: the speech-in code reports Quiet.
        assert_eq!(model.limit(), None);
        // Toggle dictation has its own mark.
        assert_eq!(
            Model::in_phase(Phase::Listening, None).with_toggle().mark(),
            Some(Mark::Toggle)
        );
    }

    #[test]
    fn a_start_stops_the_speech_that_plays() {
        let mut model = Model::default();
        model.apply(speak("t2"));
        let effects = model.apply(started("t1"));
        assert_eq!(
            effects,
            vec![
                Effect::StopSpeech {
                    terminal: Some("t2".into())
                },
                Effect::Listen { ticket: 1 }
            ]
        );
        assert_eq!(model.phase(), Phase::Listening);
    }

    #[test]
    fn a_turn_goes_from_listening_to_transcribing_to_waiting() {
        let mut model = conversing();
        assert!(model.apply(turn(1, Turn::Heard)).is_empty());
        assert_eq!(model.phase(), Phase::Transcribing);
        assert!(model.apply(turn(1, Turn::Sent)).is_empty());
        assert_eq!(model.phase(), Phase::Waiting);
        assert_eq!(model.mark(), Some(Mark::Conversation));
        // The pill waits for the pane with no frame, and gives up after 30 min.
        assert!(!Phase::Waiting.animated());
        assert_eq!(model.limit(), Some(WAITING_LIMIT));
        // A report on an old turn does nothing.
        assert!(model.apply(turn(9, Turn::Failed("late".into()))).is_empty());
        assert_eq!(model.phase(), Phase::Waiting);
    }

    #[test]
    fn the_reply_is_spoken_and_then_the_pill_listens_again() {
        let mut model = waiting();
        // Another pane does not speak in this conversation.
        assert!(model.apply(speak("t2")).is_empty());
        assert_eq!(model.phase(), Phase::Waiting);
        let effects = model.apply(speak("t1"));
        assert_eq!(model.phase(), Phase::Preparing);
        assert!(matches!(effects[..], [Effect::StartSpeech { .. }]));
        // A reply of several parts: only the last one ends the speech.
        model.apply(speak("t1"));
        model.apply(speech(1, SpeechEvent::Playing));
        assert_eq!(model.phase(), Phase::Speaking);
        assert!(model.apply(speech(1, SpeechEvent::Done)).is_empty());
        assert_eq!(model.phase(), Phase::Speaking);
        let effects = model.apply(speech(2, SpeechEvent::Done));
        assert_eq!(effects, vec![Effect::Listen { ticket: 2 }]);
        assert_eq!(model.phase(), Phase::Listening);
        // The second turn is turn 2: a report on turn 1 does nothing now.
        assert!(model.apply(turn(1, Turn::Heard)).is_empty());
        assert_eq!(model.phase(), Phase::Listening);
        model.apply(turn(2, Turn::Heard));
        assert_eq!(model.phase(), Phase::Transcribing);
    }

    #[test]
    fn a_microphone_that_is_busy_hears_no_speech_of_the_pane() {
        // A reply that comes while the pill transcribes a turn is not spoken.
        let mut model = conversing();
        model.apply(turn(1, Turn::Heard));
        assert!(model.apply(speak("t1")).is_empty());
        assert_eq!(model.phase(), Phase::Transcribing);
    }

    #[test]
    fn a_reply_with_nothing_to_speak_makes_the_pill_listen_again() {
        let mut model = waiting();
        let other = Input::Converse(Converse::SilentReply {
            terminal: "t2".into(),
        });
        assert!(model.apply(other).is_empty());
        let silent = Input::Converse(Converse::SilentReply {
            terminal: "t1".into(),
        });
        assert_eq!(model.apply(silent), vec![Effect::Listen { ticket: 2 }]);
        assert_eq!(model.phase(), Phase::Listening);
        // Not waiting: nothing to do.
        let again = Input::Converse(Converse::SilentReply {
            terminal: "t1".into(),
        });
        assert!(model.apply(again).is_empty());
    }

    #[test]
    fn a_turn_with_no_text_listens_again_and_sends_nothing() {
        let mut model = conversing();
        model.apply(turn(1, Turn::Heard));
        assert_eq!(
            model.apply(turn(1, Turn::Nothing)),
            vec![Effect::Listen { ticket: 2 }]
        );
        assert_eq!(model.phase(), Phase::Listening);
    }

    #[test]
    fn the_word_stop_ends_the_mode_when_the_core_answers() {
        let mut model = conversing();
        model.apply(turn(1, Turn::Heard));
        assert_eq!(
            model.apply(turn(1, Turn::Stop)),
            vec![Effect::EndConversation]
        );
        let effects = model.apply(Input::Converse(Converse::Stopped));
        assert_eq!(model.phase(), Phase::Hidden);
        assert_eq!(model.conversation(), None);
        assert_eq!(model.mark(), None);
        assert_eq!(effects, vec![Effect::CancelRecording]);
        // A second stop does nothing.
        assert!(model.apply(Input::Converse(Converse::Stopped)).is_empty());
    }

    #[test]
    fn a_stop_during_the_speech_silences_it() {
        let mut model = waiting();
        model.apply(speak("t1"));
        let effects = model.apply(Input::Converse(Converse::Stopped));
        assert_eq!(model.phase(), Phase::Hidden);
        assert_eq!(
            effects,
            vec![
                Effect::StopSpeech {
                    terminal: Some("t1".into())
                },
                Effect::CancelRecording
            ]
        );
    }

    #[test]
    fn a_click_asks_the_core_to_end_the_mode_in_every_phase() {
        for model in [conversing(), waiting()] {
            let mut model = model;
            assert_eq!(model.apply(Input::Click), vec![Effect::EndConversation]);
        }
        let mut model = waiting();
        model.apply(speak("t1"));
        assert_eq!(model.apply(Input::Click), vec![Effect::EndConversation]);
        // The click on an error line hides it, as always.
        let mut model = Model::default();
        model.apply(Input::Fail("x".into()));
        model.apply(Input::Click);
        assert_eq!(model.phase(), Phase::Hidden);
    }

    #[test]
    fn a_new_prompt_in_the_pane_during_the_speech_goes_back_to_waiting() {
        let mut model = waiting();
        model.apply(speak("t1"));
        model.apply(speech(1, SpeechEvent::Playing));
        let effects = model.apply(Input::CoreStop {
            terminal: "t1".into(),
        });
        assert_eq!(model.phase(), Phase::Waiting);
        assert_eq!(
            effects,
            vec![Effect::StopSpeech {
                terminal: Some("t1".into())
            }]
        );
    }

    #[test]
    fn ten_minutes_with_no_turn_end_the_mode_with_a_line() {
        let mut model = conversing();
        assert!(model.apply(Input::Quiet { ticket: 9 }).is_empty());
        let effects = model.apply(Input::Quiet { ticket: 1 });
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(
            model.error(),
            Some("Conversation stopped: no turn for 10 min")
        );
        assert_eq!(
            effects,
            vec![Effect::CancelRecording, Effect::EndConversation]
        );
        assert_eq!(model.conversation(), None);
        assert_eq!(model.mark(), None);
        // The answer of the core finds no mode, and a second failure asks for no second stop.
        assert!(model.apply(Input::Converse(Converse::Stopped)).is_empty());
        assert_eq!(model.phase(), Phase::Error);
        let effects = model.apply(Input::Fail(link_failed()));
        assert!(!effects.contains(&Effect::EndConversation));
    }

    fn link_failed() -> String {
        "The core did not stop conversation mode".into()
    }

    #[test]
    fn a_pane_that_gives_no_reply_for_30_minutes_ends_the_mode() {
        let mut model = waiting();
        let generation = model.generation();
        let effects = model.apply(Input::Timeout { generation });
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(
            model.error(),
            Some("Conversation stopped: no reply from the pane")
        );
        assert!(effects.contains(&Effect::EndConversation));
    }

    #[test]
    fn a_failed_turn_or_speech_ends_the_mode() {
        let mut model = conversing();
        model.apply(turn(1, Turn::Heard));
        let effects = model.apply(turn(1, Turn::Failed("The pane closed".into())));
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(model.error(), Some("The pane closed"));
        assert_eq!(
            effects,
            vec![Effect::CancelRecording, Effect::EndConversation]
        );
        let mut model = waiting();
        model.apply(speak("t1"));
        let effects = model.apply(speech(1, SpeechEvent::Failed("no voice".into())));
        assert_eq!(model.phase(), Phase::Error);
        assert!(effects.contains(&Effect::EndConversation));
        assert!(effects.contains(&Effect::StopSpeech {
            terminal: Some("t1".into())
        }));
    }

    #[test]
    fn a_start_with_no_pane_shows_a_line_only_when_the_pill_is_idle() {
        let mut model = Model::default();
        model.apply(Input::Converse(Converse::NoPane));
        assert_eq!(model.phase(), Phase::Error);
        assert_eq!(model.error(), Some("Focus a pane first"));
        // A mode that runs is not stopped by it.
        let mut model = waiting();
        assert!(model.apply(Input::Converse(Converse::NoPane)).is_empty());
        assert_eq!(model.phase(), Phase::Waiting);
        // Neither is a dictation.
        let mut model = Model::in_phase(Phase::Listening, None);
        assert!(model.apply(Input::Converse(Converse::NoPane)).is_empty());
    }

    #[test]
    fn a_broken_link_ends_the_mode_for_the_pill() {
        let mut model = waiting();
        let effects = model.apply(Input::Disconnected);
        assert_eq!(model.phase(), Phase::Hidden);
        assert_eq!(model.conversation(), None);
        assert_eq!(effects, vec![Effect::CancelRecording]);
    }

    #[test]
    fn a_start_for_another_pane_replaces_the_mode() {
        let mut model = waiting();
        let effects = model.apply(started("t2"));
        assert_eq!(model.conversation(), Some("t2"));
        assert_eq!(effects, vec![Effect::Listen { ticket: 2 }]);
        // The turn of the old pane does nothing.
        assert!(model.apply(turn(1, Turn::Sent)).is_empty());
        assert_eq!(model.phase(), Phase::Listening);
    }

    #[test]
    fn only_four_phases_animate() {
        let animated: Vec<_> = Phase::ALL.into_iter().filter(|p| p.animated()).collect();
        assert_eq!(
            animated,
            [
                Phase::Preparing,
                Phase::Speaking,
                Phase::Listening,
                Phase::Transcribing
            ]
        );
    }

    #[test]
    fn phase_names_round_trip() {
        for phase in Phase::ALL {
            assert_eq!(Phase::parse(phase.name()), Some(phase));
        }
        assert_eq!(Phase::parse("loud"), None);
    }
}
