//! The armory menu, shown before each match.
//!
//! Two views:
//! - **Fighters:** for each class, the loadout and the upgrade every
//!   fighter of that class comes with. Extras add to the fighter's price.
//! - **Machine weaponry:** bring up to three of mines, bombs, turrets,
//!   artillery and air strikes, and pick each one's variant.
//!
//! START MATCH begins the setup phase with that armory. During a match,
//! Esc comes back here (and ends the match); N starts a new map with the
//! same armory.

use bevy::prelude::*;

use crate::faction::LocalPlayer;
use crate::game::{MatchEntity, end_match, initial_seed, start_match};
use crate::sim::armory::{Armory, MACHINE_SLOTS, Machine, Modifier, loadout_blurb, loadout_cost};
use crate::sim::classes::{FighterClass, Loadout};
use crate::sim::economy::base_cost;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuState>()
            .init_resource::<PlayerArmory>()
            .add_systems(Startup, spawn_menu)
            .add_systems(Update, (menu_clicks, refresh_menu, escape_to_menu).chain());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Fighters,
    Machines,
}

#[derive(Resource)]
pub struct MenuState {
    pub open: bool,
    pub tab: Tab,
    /// Matches started so far (the first uses the command-line seed).
    started: u32,
}

impl Default for MenuState {
    fn default() -> Self {
        Self { open: true, tab: Tab::Fighters, started: 0 }
    }
}

/// The armory the player has chosen (kept between matches).
#[derive(Resource, Default)]
pub struct PlayerArmory(pub Armory);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
enum MenuButton {
    Tab(Tab),
    Loadout(FighterClass, Loadout),
    Modifier(FighterClass, Modifier),
    Bring(Machine),
    Variant(Machine, bool),
    Start,
}

#[derive(Component)]
struct MenuRoot;

#[derive(Component)]
struct Page(Tab);

/// Price line under a class name.
#[derive(Component)]
struct ClassPrice(FighterClass);

#[derive(Component)]
struct SlotText;

#[derive(Component)]
struct DescText;

const IDLE: Color = Color::srgb(0.18, 0.19, 0.23);
const HOVER: Color = Color::srgb(0.28, 0.29, 0.35);
const CHOSEN: Color = Color::srgb(0.75, 0.6, 0.15);
const OFF: Color = Color::srgb(0.12, 0.12, 0.14);
const MUTED: Color = Color::srgb(0.6, 0.62, 0.68);
const HELP: &str = "Hover an option to see what it does.";

fn button(kind: MenuButton, label: String) -> impl Bundle {
    (
        Button,
        kind,
        Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
            margin: UiRect::all(Val::Px(3.0)),
            ..default()
        },
        BackgroundColor(IDLE),
        children![(Text::new(label), TextFont::from_font_size(14.0), TextColor(Color::WHITE))],
    )
}

fn caption(text: &str) -> impl Bundle {
    (
        Text::new(text),
        TextFont::from_font_size(13.0),
        TextColor(MUTED),
        Node { margin: UiRect::new(Val::Px(12.0), Val::Px(4.0), Val::Px(8.0), Val::Px(0.0)), ..default() },
    )
}

fn loadout_label(l: Loadout) -> String {
    let c = loadout_cost(l);
    if c > 0.0 { format!("{} +{c:.0}", l.name()) } else { l.name().to_string() }
}

fn modifier_label(m: Modifier) -> String {
    let c = m.cost();
    if c > 0.0 { format!("{} +{c:.0}", m.name()) } else { m.name().to_string() }
}

fn spawn_menu(mut commands: Commands) {
    let root = commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(28.0), Val::Px(20.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgb(0.05, 0.06, 0.08)),
            GlobalZIndex(50),
            Interaction::default(),
        ))
        .id();

    commands.entity(root).with_children(|root| {
        root.spawn((Text::new("TERMBOTS vs HUMANS"), TextFont::from_font_size(30.0), TextColor(Color::WHITE)));
        root.spawn((
            Text::new("ARMORY: choose what your army brings to the battle. The enemy won't know until they see it."),
            TextFont::from_font_size(15.0),
            TextColor(MUTED),
            Node { margin: UiRect::bottom(Val::Px(12.0)), ..default() },
        ));
        root.spawn(Node { flex_direction: FlexDirection::Row, margin: UiRect::bottom(Val::Px(8.0)), ..default() })
            .with_children(|row| {
                row.spawn(button(MenuButton::Tab(Tab::Fighters), "Fighters".into()));
                row.spawn(button(MenuButton::Tab(Tab::Machines), "Machine weaponry".into()));
            });

        // Fighters view.
        root.spawn((Page(Tab::Fighters), Node { flex_direction: FlexDirection::Column, ..default() }))
            .with_children(|page| {
                for class in FighterClass::ALL {
                    page.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        align_items: AlignItems::Center,
                        margin: UiRect::bottom(Val::Px(6.0)),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(Node { flex_direction: FlexDirection::Column, width: Val::Px(170.0), ..default() })
                            .with_children(|name| {
                                name.spawn((
                                    Text::new(class.stats().name),
                                    TextFont::from_font_size(17.0),
                                    TextColor(Color::WHITE),
                                ));
                                name.spawn((
                                    ClassPrice(class),
                                    Text::new(""),
                                    TextFont::from_font_size(12.0),
                                    TextColor(MUTED),
                                ));
                            });
                        row.spawn(caption("Loadout"));
                        // Fixed width, so the upgrade columns line up.
                        row.spawn(Node { width: Val::Px(390.0), flex_direction: FlexDirection::Row, ..default() })
                            .with_children(|group| {
                                for &l in Loadout::options(class) {
                                    group.spawn(button(MenuButton::Loadout(class, l), loadout_label(l)));
                                }
                            });
                        row.spawn(caption("Upgrade"));
                        for m in Modifier::ALL {
                            row.spawn(button(MenuButton::Modifier(class, m), modifier_label(m)));
                        }
                    });
                }
                page.spawn((
                    Text::new(
                        "Every fighter of a class comes with its loadout and upgrade. The first loadout is free; \
                         extras are added to each fighter's price.",
                    ),
                    TextFont::from_font_size(13.0),
                    TextColor(MUTED),
                ));
            });

        // Machine weaponry view.
        root.spawn((Page(Tab::Machines), Node { flex_direction: FlexDirection::Column, ..default() }))
            .with_children(|page| {
                page.spawn((
                    SlotText,
                    Text::new(""),
                    TextFont::from_font_size(15.0),
                    TextColor(Color::WHITE),
                    Node { margin: UiRect::bottom(Val::Px(6.0)), ..default() },
                ));
                for machine in Machine::ALL {
                    page.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        margin: UiRect::bottom(Val::Px(6.0)),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(Node { width: Val::Px(170.0), ..default() }).with_children(|b| {
                            b.spawn(button(MenuButton::Bring(machine), String::new()));
                        });
                        row.spawn(caption("Variant"));
                        for alt in [false, true] {
                            let label = format!("{} {:.0}", machine.variant_name(alt), machine.price(alt));
                            row.spawn(button(MenuButton::Variant(machine, alt), label));
                        }
                    });
                }
                page.spawn((
                    Text::new(
                        "Mines, bombs, turrets and artillery can be hidden in your half during setup, or built by \
                         engineers during battle. Turrets and artillery fire on their own. Air strikes are the only \
                         machine weapon you aim.",
                    ),
                    TextFont::from_font_size(13.0),
                    TextColor(MUTED),
                ));
            });

        // What the hovered option does.
        root.spawn((
            Node {
                margin: UiRect::top(Val::Px(14.0)),
                padding: UiRect::all(Val::Px(10.0)),
                min_height: Val::Px(44.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.1, 0.11, 0.14)),
            children![(DescText, Text::new(HELP), TextFont::from_font_size(14.0), TextColor(Color::WHITE))],
        ));

        root.spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            margin: UiRect::top(Val::Px(14.0)),
            ..default()
        })
        .with_children(|row| {
            row.spawn(button(MenuButton::Start, "START MATCH".into()));
            row.spawn((
                Text::new("   In a match, Esc returns here (ending the match). N starts a new map with the same armory."),
                TextFont::from_font_size(13.0),
                TextColor(MUTED),
            ));
        });
    });
}

fn describe(kind: MenuButton, armory: &Armory) -> String {
    match kind {
        MenuButton::Tab(Tab::Fighters) => "Loadouts and upgrades for each fighter class.".into(),
        MenuButton::Tab(Tab::Machines) => format!("Pick up to {MACHINE_SLOTS} kinds of machine weaponry to bring."),
        MenuButton::Loadout(class, l) => format!("{} - {}: {}", class.stats().name, l.name(), loadout_blurb(l)),
        MenuButton::Modifier(class, m) => format!("{} - {}: {}", class.stats().name, m.name(), m.blurb()),
        MenuButton::Bring(m) => {
            let alt = armory.variant(m).unwrap_or(false);
            format!("{}: {}", m.name(), m.variant_blurb(alt))
        }
        MenuButton::Variant(m, alt) => format!("{} ({:.0}): {}", m.variant_name(alt), m.price(alt), m.variant_blurb(alt)),
        MenuButton::Start => "Start the setup phase with this armory.".into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn menu_clicks(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    local: Res<LocalPlayer>,
    mut state: ResMut<MenuState>,
    mut armory: ResMut<PlayerArmory>,
    buttons: Query<(&Interaction, &MenuButton), Changed<Interaction>>,
    mut desc: Single<&mut Text, With<DescText>>,
) {
    if !state.open {
        return;
    }
    for (interaction, kind) in &buttons {
        match interaction {
            Interaction::Hovered => desc.0 = describe(*kind, &armory.0),
            Interaction::None => {}
            Interaction::Pressed => match *kind {
                MenuButton::Tab(tab) => state.tab = tab,
                MenuButton::Loadout(class, l) => armory.0.kit_mut(class).loadout = l,
                MenuButton::Modifier(class, m) => armory.0.kit_mut(class).modifier = m,
                MenuButton::Bring(m) => {
                    if !armory.0.toggle(m) {
                        desc.0 = format!("You can bring {MACHINE_SLOTS} kinds of machine weaponry. Leave one first.");
                    }
                }
                MenuButton::Variant(m, alt) => {
                    if !armory.0.brings(m) && !armory.0.toggle(m) {
                        desc.0 = format!("You can bring {MACHINE_SLOTS} kinds of machine weaponry. Leave one first.");
                    }
                    armory.0.set_variant(m, alt);
                }
                MenuButton::Start => {
                    // The first match uses the command-line seed (if any).
                    let seed = if state.started == 0 {
                        initial_seed()
                    } else {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(1, |d| d.as_millis() as u64 % 100_000)
                    };
                    state.started += 1;
                    state.open = false;
                    desc.0 = HELP.into();
                    start_match(&mut commands, &mut images, seed, local.0, &armory.0);
                }
            },
        }
    }
}

type ButtonView<'a> = (&'a MenuButton, &'a Interaction, &'a mut BackgroundColor, &'a Children);

#[allow(clippy::type_complexity)]
fn refresh_menu(
    state: Res<MenuState>,
    armory: Res<PlayerArmory>,
    mut root: Single<&mut Node, (With<MenuRoot>, Without<Page>)>,
    mut pages: Query<(&Page, &mut Node), Without<MenuRoot>>,
    mut buttons: Query<ButtonView>,
    mut texts: Query<(&mut Text, Option<&ClassPrice>, Option<&SlotText>), Without<DescText>>,
) {
    root.display = if state.open { Display::Flex } else { Display::None };
    if !state.open {
        return;
    }
    for (page, mut node) in &mut pages {
        node.display = if page.0 == state.tab { Display::Flex } else { Display::None };
    }
    let a = &armory.0;
    for (kind, interaction, mut bg, children) in &mut buttons {
        let (chosen, enabled) = match *kind {
            MenuButton::Tab(t) => (t == state.tab, true),
            MenuButton::Loadout(c, l) => (a.kit(c).loadout == l, true),
            MenuButton::Modifier(c, m) => (a.kit(c).modifier == m, true),
            MenuButton::Bring(m) => (a.brings(m), true),
            MenuButton::Variant(m, alt) => (a.variant(m) == Some(alt), a.brings(m)),
            MenuButton::Start => (false, true),
        };
        let color = match (chosen, enabled, interaction) {
            (true, _, _) => CHOSEN,
            (false, _, Interaction::Hovered | Interaction::Pressed) => HOVER,
            (false, false, _) => OFF,
            _ => IDLE,
        };
        if bg.0 != color {
            bg.0 = color;
        }
        if let MenuButton::Bring(m) = *kind {
            let label = format!("{} {}", if a.brings(m) { "[x]" } else { "[  ]" }, m.name());
            for c in children.iter() {
                if let Ok((mut t, _, _)) = texts.get_mut(c)
                    && t.0 != label
                {
                    t.0 = label.clone();
                }
            }
        }
    }
    for (mut text, price, slots) in &mut texts {
        let new = if let Some(p) = price {
            let extra = a.surcharge(p.0);
            if extra > 0.0 {
                format!("{:.0} + {:.0} kit = {:.0} each", base_cost(p.0), extra, base_cost(p.0) + extra)
            } else {
                format!("{:.0} each", base_cost(p.0))
            }
        } else if slots.is_some() {
            format!("Machine slots used: {} / {MACHINE_SLOTS}", a.machines.len())
        } else {
            continue;
        };
        if text.0 != new {
            text.0 = new;
        }
    }
}

/// Esc during a match: back to the armory (the match ends).
fn escape_to_menu(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut state: ResMut<MenuState>,
    mut time: ResMut<Time<Virtual>>,
    entities: Query<Entity, With<MatchEntity>>,
) {
    if state.open || !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    end_match(&mut commands, entities.iter());
    time.unpause();
    state.open = true;
}
