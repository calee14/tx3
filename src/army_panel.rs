//! The army panel (right side): build a platoon of 1-5 fighters, then
//! click the map to place it (setup) or deploy it (battle).
//!
//! Setup: place platoons anywhere in your half, undo the last one, use the
//! default army, and start the battle when ready.
//! Battle: pick "On foot" (walk from base) or "Vehicle" (+150 Supply,
//! rides to the spot and parks there as cover), then click an explored tile.

use bevy::prelude::*;

use crate::commander::default_army_commands;
use crate::faction::{Faction, LocalPlayer};
use crate::sim::classes::FighterClass;
use crate::sim::economy::{
    MatchPhase, SUPPLY_PER_SECOND, Supply, VEHICLE_COST, count_classes, fighter_cost, platoon_cost,
};
use crate::sim::fighter::Fighter;
use crate::sim::platoon::MAX_PLATOON_SIZE;
use crate::sim::vehicle::Vehicle;
use crate::sim::{CommandQueue, SimCommand};

pub struct ArmyPanelPlugin;

impl Plugin for ArmyPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArmyBuilder>()
            .add_systems(Startup, spawn_panel)
            .add_systems(
                Update,
                (panel_buttons, update_panel).chain().run_if(resource_exists::<MatchPhase>),
            );
    }
}

/// The platoon being put together in the panel.
#[derive(Resource, Default, Debug)]
pub struct ArmyBuilder {
    pub classes: Vec<FighterClass>,
    pub vehicle: bool,
}

impl ArmyBuilder {
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelButton {
    Add(FighterClass),
    RemoveLast,
    Clear,
    Transport,
    DefaultArmy,
    Undo,
    StartBattle,
}

impl PanelButton {
    fn setup_only(self) -> bool {
        matches!(self, PanelButton::DefaultArmy | PanelButton::Undo | PanelButton::StartBattle)
    }
    fn battle_only(self) -> bool {
        matches!(self, PanelButton::Transport)
    }
}

/// Two-line header (phase + Supply), above the buttons.
#[derive(Component)]
struct PanelHeader;

/// Details below the buttons (so the buttons never move).
#[derive(Component)]
struct PanelText;

#[derive(Component)]
struct ButtonLabel;

const IDLE: Color = Color::srgb(0.2, 0.2, 0.24);
const HOVER: Color = Color::srgb(0.32, 0.32, 0.38);
const ACTIVE: Color = Color::srgb(0.75, 0.6, 0.15);

fn button(kind: PanelButton) -> impl Bundle {
    (
        Button,
        kind,
        Node {
            padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
            margin: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(IDLE),
        children![(ButtonLabel, Text::new(""), TextFont::from_font_size(13.0), TextColor(Color::WHITE))],
    )
}

fn row() -> Node {
    Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, ..default() }
}

fn spawn_panel(mut commands: Commands) {
    use FighterClass::*;
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(8.0),
            top: Val::Px(8.0),
            width: Val::Px(290.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        // Lets the panel block clicks from reaching the map.
        Interaction::default(),
        children![
            (PanelHeader, Text::new("\n"), TextFont::from_font_size(14.0), TextColor(Color::WHITE)),
            (row(), children![button(PanelButton::Add(Assaulter)), button(PanelButton::Add(Sniper))]),
            (row(), children![button(PanelButton::Add(HeavyWeapons)), button(PanelButton::Add(Juggernaut))]),
            (row(), children![button(PanelButton::RemoveLast), button(PanelButton::Clear)]),
            (row(), children![button(PanelButton::Transport)]),
            (row(), children![button(PanelButton::DefaultArmy), button(PanelButton::Undo)]),
            (row(), children![button(PanelButton::StartBattle)]),
            (
                Node { margin: UiRect::top(Val::Px(6.0)), ..default() },
                children![(PanelText, Text::new(""), TextFont::from_font_size(13.0), TextColor(Color::WHITE))]
            ),
        ],
    ));
}

fn panel_buttons(
    local: Res<LocalPlayer>,
    phase: Res<MatchPhase>,
    mut builder: ResMut<ArmyBuilder>,
    mut queue: ResMut<CommandQueue>,
    buttons: Query<(&Interaction, &PanelButton), Changed<Interaction>>,
) {
    for (interaction, kind) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *kind {
            PanelButton::Add(class) => {
                if builder.classes.len() < MAX_PLATOON_SIZE {
                    builder.classes.push(class);
                }
            }
            PanelButton::RemoveLast => {
                builder.classes.pop();
            }
            PanelButton::Clear => builder.classes.clear(),
            PanelButton::Transport => builder.vehicle = !builder.vehicle,
            PanelButton::DefaultArmy if !phase.battle => {
                for c in default_army_commands(local.0) {
                    queue.push(c);
                }
            }
            PanelButton::Undo if !phase.battle => queue.push(SimCommand::UndoPlacement { issuer: local.0 }),
            PanelButton::StartBattle if !phase.battle => {
                builder.classes.clear();
                queue.push(SimCommand::Ready { issuer: local.0 });
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_panel(
    local: Res<LocalPlayer>,
    phase: Res<MatchPhase>,
    supply: Res<Supply>,
    builder: Res<ArmyBuilder>,
    fighters: Query<(&Faction, &Fighter)>,
    vehicles: Query<(&Faction, &Vehicle)>,
    mut text: Single<&mut Text, (With<PanelText>, Without<ButtonLabel>, Without<PanelHeader>)>,
    mut header: Single<&mut Text, (With<PanelHeader>, Without<ButtonLabel>, Without<PanelText>)>,
    mut buttons: Query<(&PanelButton, &Interaction, &mut BackgroundColor, &mut Node, &Children)>,
    mut labels: Query<&mut Text, With<ButtonLabel>>,
) {
    let me = local.0;
    let counts = count_classes(me, fighters.iter(), vehicles.iter());
    let vehicle = phase.battle && builder.vehicle;
    let cost = platoon_cost(&builder.classes, &counts, vehicle);
    let have = supply.of(me);

    header.0 = if phase.battle {
        format!("REINFORCEMENTS\nSupply {:.0}   (+{SUPPLY_PER_SECOND:.0}/s)", have)
    } else if phase.ready[me.index()] {
        "SETUP\nWaiting for the enemy...".to_string()
    } else {
        format!("SETUP: place your army (hidden)\nSupply {:.0}", have)
    };

    let mut lines = Vec::new();
    if builder.is_empty() {
        lines.push(format!("New platoon: add 1-{MAX_PLATOON_SIZE} fighters"));
    } else {
        let names: Vec<&str> = builder.classes.iter().map(|c| c.stats().letter).collect();
        lines.push(format!("New platoon: {}  ({}/{MAX_PLATOON_SIZE})", names.join(" "), builder.classes.len()));
        lines.push(format!(
            "Cost {:.0}{}{}",
            cost,
            if vehicle { " incl. vehicle" } else { "" },
            if cost > have { "  - not enough Supply" } else { "" }
        ));
        lines.push(
            if phase.battle {
                if builder.vehicle {
                    "Click an explored tile: a vehicle drives them there\nand parks as cover. Right-click cancels."
                } else {
                    "Click an explored tile: they walk there from base.\nRight-click cancels."
                }
            } else {
                "Click inside your half to place it. Right-click cancels."
            }
            .to_string(),
        );
    }
    lines.push("Prices rise 25% per fighter of a class you already have.".to_string());
    text.0 = lines.join("\n");

    // Next price of each class, given what's already in the builder.
    let mut with_builder = counts;
    for &c in &builder.classes {
        with_builder.add(c);
    }

    for (kind, interaction, mut bg, mut node, children) in &mut buttons {
        let hidden = (kind.setup_only() && phase.battle) || (kind.battle_only() && !phase.battle);
        node.display = if hidden { Display::None } else { Display::Flex };
        let active = *kind == PanelButton::Transport && builder.vehicle;
        bg.0 = match (active, interaction) {
            (true, _) => ACTIVE,
            (false, Interaction::Hovered | Interaction::Pressed) => HOVER,
            _ => IDLE,
        };
        let label = match *kind {
            PanelButton::Add(class) => {
                format!("+ {} {:.0}", class.stats().name, fighter_cost(class, &with_builder))
            }
            PanelButton::RemoveLast => "Remove last".into(),
            PanelButton::Clear => "Clear".into(),
            PanelButton::Transport => {
                if builder.vehicle {
                    format!("Transport: Vehicle (+{VEHICLE_COST:.0})")
                } else {
                    "Transport: On foot".into()
                }
            }
            PanelButton::DefaultArmy => "Place default army".into(),
            PanelButton::Undo => "Undo last placement".into(),
            PanelButton::StartBattle => "START BATTLE".into(),
        };
        for child in children.iter() {
            if let Ok(mut t) = labels.get_mut(child)
                && t.0 != label
            {
                t.0 = label.clone();
            }
        }
    }
}
