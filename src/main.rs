use tcod::colors::*;
use tcod::console::*;

use tcod::map::{FovAlgorithm, Map as FovMap};

use tcod::input::{self, Event, Key, Mouse};

use rand::Rng;

use std::cmp;

use std::error::Error;
use std::fs::File;
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

const SCREEN_WIDTH: i32 = 80;
const SCREEN_HEIGHT: i32 = 50;

const LIMIT_FPS: i32 = 20;

const MAP_WIDTH: i32 = 80;
const MAP_HEIGHT: i32 = 43;

const BAR_WIDTH: i32 = 20;
const PANEL_HEIGHT: i32 = SCREEN_HEIGHT - MAP_HEIGHT;
const PANEL_Y: i32 = SCREEN_HEIGHT - PANEL_HEIGHT;

const INVENTORY_WIDTH: i32 = 50;

const CHOOSE_MENU_WIDTH: i32 = 50;

const LEVEL_SCREEN_WIDTH: i32 = 40;

const CHARACTER_SCREEN_WIDTH: i32 = 30;

const MSG_X: i32 = BAR_WIDTH + 2;
const MSG_WIDTH: i32 = SCREEN_WIDTH - BAR_WIDTH - 2;
const MSG_HEIGHT: usize = PANEL_HEIGHT as usize - 1;

const COLOR_DARK_WALL: Color = Color { r: 0, g: 0, b: 100 };
const COLOR_LIGHT_WALL: Color = Color {
    r: 130,
    g: 110,
    b: 50,
};

const COLOR_DARK_GROUND: Color = Color {
    r: 50,
    g: 50,
    b: 150,
};
const COLOR_LIGHT_GROUND: Color = Color {
    r: 200,
    g: 180,
    b: 50,
};

const MANA_REGENERATION: i32 = 15;

const PLAYER: usize = 0;

const DAMAGE_SCALING: f32 = 0.5;
const DAMAGE_OFFSET: f32 = -0.5;

const LEVEL_UP_BASE: i32 = 350;
const LEVEL_UP_FACTOR: i32 = 150;

const CONFUSED_NUM_TURNS: i32 = 10;
const CONFUSED_RANGE: i32 = 8;

const ROOM_MAX_SIZE: i32 = 10;
const ROOM_MIN_SIZE: i32 = 6;
const MAX_ROOMS: i32 = 30;

const FOV_ALGO: FovAlgorithm = FovAlgorithm::Basic;
const FOV_LIGHT_WALLS: bool = true;
const TORCH_RADIUS: i32 = 10;

const HEAL_AMOUNT: i32 = 40;
const LIGHTNING_DAMAGE: i32 = 40;
const LIGHTNING_RANGE: i32 = 5;

const FIREBALL_RANGE: i32 = 8;
const FIREBALL_CHARGES: i32 = 3;
const FIREBALL_DAMAGE: i32 = 25;

fn calculate_damage(base_damage: f32, defense: f32) -> i32 {
    let sigmoid = |x: f32| -> f32 { 1.0 / (1.0 + (-x).exp()) };

    let damage = base_damage * sigmoid((base_damage - defense) * DAMAGE_SCALING + DAMAGE_OFFSET);

    let rest = if rand::thread_rng().gen::<f32>() < damage.fract() {
        1
    } else {
        0
    };

    damage.floor() as i32 + rest
}

#[derive(Serialize, Deserialize)]
struct Messages {
    messages: Vec<(String, Color)>,
}

impl Messages {
    pub fn new() -> Self {
        Self { messages: vec![] }
    }

    pub fn add<T: Into<String>>(&mut self, message: T, color: Color) {
        self.messages.push((message.into(), color));
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &(String, Color)> {
        self.messages.iter()
    }
}

struct Tcod {
    root: Root,
    con: Offscreen,
    panel: Offscreen,
    fov: FovMap,
    key: Key,
    mouse: Mouse,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Tile {
    blocked: bool,
    block_sight: bool,
    explored: bool,
}

impl Tile {
    pub fn empty() -> Self {
        Tile {
            blocked: false,
            block_sight: false,
            explored: false,
        }
    }

    pub fn wall() -> Self {
        Tile {
            blocked: true,
            block_sight: true,
            explored: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect {
            x1: x,
            x2: x + w,
            y1: y,
            y2: y + h,
        }
    }

    pub fn center(&self) -> (i32, i32) {
        let center_x = (self.x1 + self.x2) / 2;
        let center_y = (self.y1 + self.y2) / 2;

        (center_x, center_y)
    }

    pub fn intersects_with(&self, other: &Rect) -> bool {
        (self.x1 <= other.x2)
            && (self.x2 >= other.x1)
            && (self.y1 <= other.y2)
            && (self.y2 >= other.y1)
    }
}

fn create_room(room: Rect, map: &mut Map) {
    for x in (room.x1 + 1)..room.x2 {
        for y in (room.y1 + 1)..room.y2 {
            map[x as usize][y as usize] = Tile::empty();
        }
    }
}

struct Transition {
    level: u32,
    value: u32,
}

fn from_dungeon_level(table: &[Transition], level: u32) -> u32 {
    table
        .iter()
        .rev()
        .find(|transition| level >= transition.level)
        .map_or(0, |transition| transition.value)
}

fn get_equipped_in_slot(slot: Slot, inventory: &[Object]) -> Option<usize> {
    for (inventory_id, item) in inventory.iter().enumerate() {
        if item
            .equipment
            .as_ref()
            .map_or(false, |e| e.equipped && e.slot == slot)
        {
            return Some(inventory_id);
        }
    }
    None
}

fn place_objects(room: Rect, map: &Map, objects: &mut Vec<Object>, level: u32) {
    use rand::distributions::{IndependentSample, Weighted, WeightedChoice};

    let max_monsters = from_dungeon_level(
        &[
            Transition { level: 0, value: 2 },
            Transition { level: 4, value: 3 },
            Transition { level: 6, value: 5 },
            Transition { level: 9, value: 8 },
            Transition {
                level: 14,
                value: 13,
            },
        ],
        level,
    );

    let num_monsters = rand::thread_rng().gen_range(0, max_monsters + 1);

    let monster_chances = &mut [
        Weighted {
            weight: from_dungeon_level(
                &[
                    Transition {
                        level: 3,
                        value: 15,
                    },
                    Transition {
                        level: 5,
                        value: 30,
                    },
                    Transition {
                        level: 7,
                        value: 60,
                    },
                    Transition {
                        level: 12,
                        value: 100,
                    },
                ],
                level,
            ),
            item: "troll",
        },
        Weighted {
            weight: from_dungeon_level(
                &[
                    Transition {
                        level: 4,
                        value: 30,
                    },
                    Transition {
                        level: 6,
                        value: 80,
                    },
                    Transition {
                        level: 8,
                        value: 90,
                    },
                ],
                level,
            ),
            item: "archer",
        },
        Weighted {
            weight: 80,
            item: "orc",
        },
    ];
    let monster_choice = WeightedChoice::new(monster_chances);

    let max_items = from_dungeon_level(
        &[
            Transition { level: 0, value: 1 },
            Transition { level: 3, value: 2 },
            Transition { level: 7, value: 4 },
            Transition { level: 9, value: 5 },
        ],
        level,
    );

    let item_chances = &mut [
        Weighted {
            weight: from_dungeon_level(&[Transition { level: 4, value: 5 }], level),
            item: Item::Sword,
        },
        Weighted {
            weight: from_dungeon_level(&[Transition { level: 6, value: 5 }], level),
            item: Item::Helmet,
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 5,
                    value: 15,
                }],
                level,
            ),
            item: Item::FireballStaff { range: 0 },
        },
        Weighted {
            weight: from_dungeon_level(
                &[
                    Transition {
                        level: 3,
                        value: 35,
                    },
                    Transition {
                        level: 6,
                        value: 40,
                    },
                ],
                level,
            ),
            item: Item::ManaPotion,
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 12,
                    value: 10,
                }],
                level,
            ),
            item: Item::BodyArmor,
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 8,
                    value: 15,
                }],
                level,
            ),
            item: Item::Shield,
        },
        Weighted {
            weight: 35,
            item: Item::Heal { amount: 0 },
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 4,
                    value: 25,
                }],
                level,
            ),
            item: Item::Lightning { damage: 0 },
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 6,
                    value: 25,
                }],
                level,
            ),
            item: Item::Fireball {
                damage: 0,
                charges: 0,
            },
        },
        Weighted {
            weight: from_dungeon_level(
                &[Transition {
                    level: 2,
                    value: 10,
                }],
                level,
            ),
            item: Item::Confuse,
        },
    ];
    let item_choice = WeightedChoice::new(item_chances);

    for _ in 0..num_monsters {
        let x = rand::thread_rng().gen_range(room.x1 + 1, room.x2);
        let y = rand::thread_rng().gen_range(room.y1 + 1, room.y2);

        if !is_blocked(x, y, map, objects) {
            let mut monster = match monster_choice.ind_sample(&mut rand::thread_rng()) {
                "orc" => {
                    let mut orc = Object::new(x, y, 'o', "orc", DESATURATED_GREEN, true);
                    orc.fighter = Some(Fighter {
                        base_max_hp: 20,
                        hp: 20,
                        base_defense: 0,
                        base_power: 4,
                        base_max_mana: 0,
                        base_spellcast_modifier: 0,
                        mana: 0,
                        xp: 35,
                        on_death: DeathCallback::Monster,
                    });
                    orc.ai = Some(Ai::Basic);
                    orc
                }
                "troll" => {
                    let mut troll = Object::new(x, y, 'T', "troll", DARKER_GREEN, true);
                    troll.fighter = Some(Fighter {
                        base_max_hp: 30,
                        hp: 30,
                        base_defense: 2,
                        base_power: 8,
                        base_max_mana: 0,
                        base_spellcast_modifier: 0,
                        mana: 0,
                        xp: 100,
                        on_death: DeathCallback::Monster,
                    });
                    troll.ai = Some(Ai::Basic);

                    troll
                }
                "archer" => {
                    let mut archer = Object::new(x, y, 'a', "archer", DARKER_GREEN, true);
                    archer.fighter = Some(Fighter {
                        base_max_hp: 15,
                        hp: 15,
                        base_defense: 1,
                        base_power: 6,
                        base_max_mana: 0,
                        base_spellcast_modifier: 0,
                        mana: 0,
                        xp: 50,
                        on_death: DeathCallback::MonsterArcher,
                    });
                    archer.ai = Some(Ai::Archer { range: 8 });

                    archer
                }
                _ => unreachable!(),
            };

            monster.alive = true;

            objects.push(monster);
        }
    }

    let num_items = rand::thread_rng().gen_range(0, max_items + 1);

    for _ in 0..num_items {
        let x = rand::thread_rng().gen_range(room.x1 + 1, room.x2);
        let y = rand::thread_rng().gen_range(room.y1 + 1, room.y2);

        if !is_blocked(x, y, map, objects) {
            objects.push(match item_choice.ind_sample(&mut rand::thread_rng()) {
                Item::Lightning { .. } => {
                    let mut object =
                        Object::new(x, y, '#', "scroll of lightning", LIGHT_YELLOW, false);
                    object.item = Some(Item::Lightning {
                        damage: LIGHTNING_DAMAGE,
                    });
                    object
                }
                Item::FireballStaff { .. } => {
                    let mut object = Object::new(x, y, '|', "fireball staff", ORANGE, false);
                    object.item = Some(Item::FireballStaff { range: 8 });
                    object.equipment = Some(Equipment {
                        equipped: false,
                        slot: Slot::RightHand,
                        power_bonus: 0,
                        defense_bonus: 0,
                        max_hp_bonus: 0,
                        mana_usage: 5,
                        spellcast_bonus: from_dungeon_level(
                            &[
                                Transition { level: 0, value: 8 },
                                Transition {
                                    level: 6,
                                    value: 13,
                                },
                            ],
                            level,
                        ) as i32,
                        max_mana_bonus: from_dungeon_level(
                            &[
                                Transition {
                                    level: 0,
                                    value: 10,
                                },
                                Transition {
                                    level: 10,
                                    value: 20,
                                },
                            ],
                            level,
                        ) as i32,
                    });
                    object
                }
                Item::Fireball { .. } => {
                    let mut object =
                        Object::new(x, y, '#', "scroll of fireball", LIGHT_YELLOW, false);
                    object.item = Some(Item::Fireball {
                        charges: FIREBALL_CHARGES,
                        damage: FIREBALL_DAMAGE,
                    });
                    object
                }
                Item::Confuse => {
                    let mut object =
                        Object::new(x, y, '#', "scroll of confusion", LIGHT_YELLOW, false);
                    object.item = Some(Item::Confuse);
                    object
                }
                Item::Heal { .. } => {
                    let mut object = Object::new(x, y, '!', "healing potion", DARKER_RED, false);
                    object.item = Some(Item::Heal {
                        amount: HEAL_AMOUNT,
                    });
                    object
                }
                Item::ManaPotion { .. } => {
                    let mut object = Object::new(x, y, '!', "mana potion", VIOLET, false);
                    object.item = Some(Item::ManaPotion);
                    object
                }
                Item::Sword => {
                    let mut object = Object::new(x, y, '/', "sword", SKY, false);
                    object.item = Some(Item::Sword);
                    object.equipment = Some(Equipment {
                        mana_usage: 0,
                        equipped: false,
                        slot: Slot::RightHand,
                        power_bonus: 3,
                        defense_bonus: 0,
                        max_hp_bonus: 0,
                        max_mana_bonus: 0,
                        spellcast_bonus: 0,
                    });
                    object
                }
                Item::Shield => {
                    let mut object = Object::new(x, y, '[', "shield", DARKER_ORANGE, false);
                    object.item = Some(Item::Shield);
                    object.equipment = Some(Equipment {
                        mana_usage: 0,
                        equipped: false,
                        slot: Slot::LeftHand,
                        power_bonus: 0,
                        defense_bonus: 1,
                        max_hp_bonus: 0,
                        max_mana_bonus: 0,
                        spellcast_bonus: 0,
                    });
                    object
                }
                Item::Helmet => {
                    let mut object = Object::new(x, y, '^', "helmet", YELLOW, false);
                    object.item = Some(Item::Shield);
                    object.equipment = Some(Equipment {
                        mana_usage: 0,
                        equipped: false,
                        slot: Slot::Head,
                        power_bonus: 0,
                        defense_bonus: 2,
                        max_hp_bonus: 0,
                        max_mana_bonus: 0,
                        spellcast_bonus: 0,
                    });
                    object
                }
                Item::BodyArmor => {
                    let mut object = Object::new(x, y, '=', "body armor", GREEN, false);
                    object.item = Some(Item::BodyArmor);
                    object.equipment = Some(Equipment {
                        mana_usage: 0,
                        equipped: false,
                        slot: Slot::Body,
                        power_bonus: 0,
                        defense_bonus: 4,
                        max_hp_bonus: 0,
                        max_mana_bonus: 0,
                        spellcast_bonus: 0,
                    });
                    object
                }
            });
        }
    }
}

type Map = Vec<Vec<Tile>>;

#[derive(Serialize, Deserialize)]
struct Game {
    map: Map,
    messages: Messages,
    inventory: Vec<Object>,
    dungeon_level: u32,
}

fn make_map(objects: &mut Vec<Object>, level: u32) -> Map {
    // fill with "unblocked" tiles
    let mut map = vec![vec![Tile::wall(); MAP_HEIGHT as usize]; MAP_WIDTH as usize];

    assert_eq!(&objects[PLAYER] as *const _, &objects[0] as *const _);
    objects.truncate(1);

    let mut rooms = vec![];

    for _ in 0..MAX_ROOMS {
        let w = rand::thread_rng().gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);
        let h = rand::thread_rng().gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);

        let x = rand::thread_rng().gen_range(0, MAP_WIDTH - w);
        let y = rand::thread_rng().gen_range(0, MAP_HEIGHT - h);

        let new_room = Rect::new(x, y, w, h);

        let failed = rooms
            .iter()
            .any(|other_room| new_room.intersects_with(other_room));

        if !failed {
            create_room(new_room, &mut map);
            let (new_x, new_y) = new_room.center();

            if rooms.is_empty() {
                objects[PLAYER].set_pos(new_x, new_y);
            } else {
                place_objects(new_room, &map, objects, level);
                let (prev_x, prev_y) = rooms[rooms.len() - 1].center();

                if rand::random() {
                    create_h_tunnel(prev_x, new_x, prev_y, &mut map);
                    create_v_tunnel(prev_y, new_y, new_x, &mut map);
                } else {
                    create_h_tunnel(prev_x, new_x, new_y, &mut map);
                    create_v_tunnel(prev_y, new_y, prev_x, &mut map);
                }
            }

            rooms.push(new_room);
        }
    }

    let (last_room_x, last_room_y) = rooms[rooms.len() - 1].center();
    let mut stairs = Object::new(last_room_x, last_room_y, '<', "stairs", WHITE, false);
    stairs.always_visible = true;
    objects.push(stairs);

    map
}

fn create_h_tunnel(x1: i32, x2: i32, y: i32, map: &mut Map) {
    for x in cmp::min(x1, x2)..(cmp::max(x1, x2) + 1) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

fn create_v_tunnel(y1: i32, y2: i32, x: i32, map: &mut Map) {
    for y in cmp::min(y1, y2)..(cmp::max(y1, y2) + 1) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

fn render_all(tcod: &mut Tcod, game: &mut Game, objects: &Vec<Object>, fov_recompute: bool) {
    if fov_recompute {
        let player = &objects[PLAYER];
        tcod.fov
            .compute_fov(player.x, player.y, TORCH_RADIUS, FOV_LIGHT_WALLS, FOV_ALGO);
    }

    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let visible = tcod.fov.is_in_fov(x, y);
            let wall = game.map[x as usize][y as usize].block_sight;
            let color = match (visible, wall) {
                (false, true) => COLOR_DARK_WALL,
                (false, false) => COLOR_DARK_GROUND,
                (true, true) => COLOR_LIGHT_WALL,
                (true, false) => COLOR_LIGHT_GROUND,
            };

            let explored = &mut game.map[x as usize][y as usize].explored;
            if visible {
                *explored = true;
            }
            if *explored {
                tcod.con
                    .set_char_background(x, y, color, BackgroundFlag::Set);
            }
        }
    }

    let mut to_draw: Vec<_> = objects
        .iter()
        .filter(|o| {
            tcod.fov.is_in_fov(o.x, o.y)
                || (o.always_visible && game.map[o.x as usize][o.y as usize].explored)
        })
        .collect();

    to_draw.sort_by(|o1, o2| o1.blocks.cmp(&o2.blocks));

    for object in &to_draw {
        object.draw(&mut tcod.con);
    }

    blit(
        &tcod.con,
        (0, 0),
        (MAP_WIDTH, MAP_HEIGHT),
        &mut tcod.root,
        (0, 0),
        1.0,
        1.0,
    );

    tcod.panel.set_default_background(BLACK);
    tcod.panel.clear();

    let hp = objects[PLAYER].fighter.map_or(0, |f| f.hp);
    let max_hp = objects[PLAYER].max_hp(game);

    render_bar(
        &mut tcod.panel,
        1,
        1,
        BAR_WIDTH,
        "HP",
        hp,
        max_hp,
        LIGHT_RED,
        DARKER_RED,
    );

    let mana = objects[PLAYER].fighter.map_or(0, |f| f.mana);
    let max_mana = objects[PLAYER].max_mana(game);

    render_bar(
        &mut tcod.panel,
        1,
        2,
        BAR_WIDTH,
        "MANA",
        mana,
        max_mana,
        LIGHT_BLUE,
        DARKER_BLUE,
    );

    tcod.panel.print_ex(
        1,
        3,
        BackgroundFlag::None,
        TextAlignment::Left,
        format!("Dungeon level: {}", game.dungeon_level),
    );

    let mut y = MSG_HEIGHT as i32;
    for &(ref msg, color) in game.messages.iter().rev() {
        let msg_height = tcod.panel.get_height_rect(MSG_X, y, MSG_WIDTH, 0, msg);
        y -= msg_height;
        if y <= 0 {
            break;
        }

        tcod.panel.set_default_foreground(color);
        tcod.panel.print_rect(MSG_X, y, MSG_WIDTH, 0, msg);
    }

    tcod.panel.set_default_foreground(LIGHT_GREY);
    tcod.panel.print_ex(
        1,
        0,
        BackgroundFlag::None,
        TextAlignment::Left,
        get_names_under_mouse(tcod.mouse, objects, &tcod.fov),
    );

    blit(
        &tcod.panel,
        (0, 0),
        (SCREEN_WIDTH, PANEL_HEIGHT),
        &mut tcod.root,
        (0, PANEL_Y),
        1.0,
        1.0,
    );
}

fn render_bar(
    panel: &mut Offscreen,
    x: i32,
    y: i32,
    total_width: i32,
    name: &str,
    value: i32,
    maximum: i32,
    bar_color: Color,
    back_color: Color,
) {
    let bar_width = (value as f32 / maximum as f32 * total_width as f32) as i32;

    panel.set_default_background(back_color);
    panel.rect(x, y, total_width, 1, false, BackgroundFlag::Screen);

    panel.set_default_background(bar_color);
    if bar_width > 0 {
        panel.rect(x, y, bar_width, 1, false, BackgroundFlag::Screen);
    }

    panel.set_default_foreground(WHITE);
    panel.print_ex(
        x + total_width / 2,
        y,
        BackgroundFlag::None,
        TextAlignment::Center,
        &format!("{}: {}/{}", name, value, maximum),
    );
}

fn closest_target(tcod: &Tcod, objects: &Vec<Object>, max_range: i32) -> Option<usize> {
    let mut closest_enemy = None;
    let mut closest_dist = (max_range + 1) as f32;

    let player_pos = objects[PLAYER].pos();

    let _: Vec<()> = objects
        .iter()
        .enumerate()
        .map(|(id, object)| {
            if (id != PLAYER)
                && object.fighter.is_some()
                && object.ai.is_some()
                && tcod.fov.is_in_fov(object.x, object.y)
            {
                let dist = object.distance_to_point(player_pos);
                if dist < closest_dist {
                    closest_enemy = Some(id);
                    closest_dist = dist;
                }
            }
        })
        .collect();

    closest_enemy
}

fn choose_target<F>(
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
    filter: &mut F,
    msg: &str,
) -> Option<usize>
where
    F: FnMut(&Tcod, &&Object) -> bool,
{
    let targets: Vec<&Object> = objects.iter().filter(|o| filter(tcod, o)).collect();

    if targets.len() == 0 {
        return None;
    }

    let target_names: Vec<_> = targets.iter().map(|o| o.name.clone()).collect();

    // do not render the inventory menu
    tcod.root.clear();
    render_all(tcod, game, objects, false);

    let target_id = menu(msg, &target_names[..], CHOOSE_MENU_WIDTH, tcod);

    if let Some(target_id) = target_id {
        for id in 0..objects.len() {
            if (&objects[id] as *const _) == (targets[target_id] as *const _) {
                return Some(id);
            }
        }
    }

    None
}

fn cast_lightning(
    inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
) -> UseResult {
    let target = closest_target(tcod, objects, LIGHTNING_RANGE);
    if let Some(target) = target {
        let (player, target) = mut_two(PLAYER, target, objects);
        player.attack(target, game, Some(inventory_id));

        UseResult::UsedUp
    } else {
        game.messages.add("You need to choose a target", WHITE);
        UseResult::Cancelled
    }
}

fn cast_fireballstaff(
    inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
) -> UseResult {
    let range = if let Some(Item::FireballStaff { range }) = game.inventory[inventory_id].item {
        range
    } else {
        0
    };
    let player_pos = objects[PLAYER].pos();
    let target = choose_target(
        tcod,
        game,
        objects,
        &mut |tcod: &Tcod, o: &&Object| -> bool {
            tcod.fov.is_in_fov(o.x, o.y)
                && o.ai.is_some()
                && o.distance_to_point(player_pos) <= range as f32
        },
        "Choose target for fireball staff",
    );
    if let Some(target) = target {
        let (player, target) = mut_two(PLAYER, target, objects);
        player.attack(target, game, Some(inventory_id));

        UseResult::Used
    } else {
        game.messages.add("You need to choose a target", WHITE);
        UseResult::Cancelled
    }
}

fn cast_fireball(
    inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
) -> UseResult {
    let player_pos = objects[PLAYER].pos();
    let target = choose_target(
        tcod,
        game,
        objects,
        &mut |tcod: &Tcod, o: &&Object| -> bool {
            tcod.fov.is_in_fov(o.x, o.y)
                && o.ai.is_some()
                && o.distance_to_point(player_pos) <= FIREBALL_RANGE as f32
        },
        "Choose target for fireball scroll",
    );
    if let Some(target) = target {
        let (player, target) = mut_two(PLAYER, target, objects);
        player.attack(target, game, Some(inventory_id));

        if let Some(Item::Fireball { charges, damage }) = game.inventory[inventory_id].item {
            game.inventory[inventory_id].item = Some(Item::Fireball {
                charges: charges - 1,
                damage,
            });
            if charges > 1 {
                UseResult::Used
            } else {
                UseResult::UsedUp
            }
        } else {
            println!("Something wierd happened, this should not happen.");
            UseResult::Cancelled
        }
    } else {
        game.messages.add("You need to choose a target", WHITE);
        UseResult::Cancelled
    }
}

fn cast_confuse(
    _inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
) -> UseResult {
    let player_pos = objects[PLAYER].pos();
    let target = choose_target(
        tcod,
        game,
        objects,
        &mut |tcod: &Tcod, o: &&Object| -> bool {
            tcod.fov.is_in_fov(o.x, o.y)
                && o.ai.is_some()
                && o.distance_to_point(player_pos) <= CONFUSED_RANGE as f32
        },
        "Choose target for confusion scroll",
    );
    if let Some(target) = target {
        let old_ai = objects[target].ai.take().unwrap_or(Ai::Basic);
        objects[target].ai = Some(Ai::Confused {
            previous_ai: Box::new(old_ai),
            num_turns: CONFUSED_NUM_TURNS,
        });
        game.messages.add(
            format!(
                "The eyes of {} look vacant, as it starts to stumble around!",
                objects[target].name
            ),
            YELLOW,
        );

        UseResult::UsedUp
    } else {
        game.messages.add("You need to choose a target", WHITE);
        UseResult::Cancelled
    }
}

fn cast_mana_potion(
    inventory_id: usize,
    _tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
) -> UseResult {
    if let Some(fighter) = objects[PLAYER].fighter {
        if fighter.mana == objects[PLAYER].max_mana(game) {
            game.messages.add("You are already at full mana", YELLOW);
            return UseResult::Cancelled;
        } else {
            if let Some(Item::ManaPotion) = game.inventory[inventory_id].item {
                game.messages.add("your arcane energy is bubbling", YELLOW);
                objects[PLAYER].regenerate_mana(MANA_REGENERATION, game);

                return UseResult::UsedUp;
            }
            println!(
                "Tried to heal with an item that is not a mana regeneration {:?}",
                game.inventory[inventory_id]
            );
        }
    }
    UseResult::Cancelled
}

fn cast_heal(
    inventory_id: usize,
    _tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
) -> UseResult {
    if let Some(fighter) = objects[PLAYER].fighter {
        if fighter.hp == objects[PLAYER].max_hp(game) {
            game.messages.add("You are already at full heal", YELLOW);
            return UseResult::Cancelled;
        } else {
            if let Some(Item::Heal { amount }) = game.inventory[inventory_id].item {
                game.messages
                    .add("your wounds start to feel better", YELLOW);
                objects[PLAYER].heal(amount, game);

                return UseResult::UsedUp;
            }
            println!(
                "Tried to heal with an item that is not a heal {:?}",
                game.inventory[inventory_id]
            );
        }
    }
    UseResult::Cancelled
}

fn pick_item_up(object_id: usize, game: &mut Game, objects: &mut Vec<Object>) {
    if game.inventory.len() >= 26 {
        game.messages.add(
            format!(
                "Cannot pick up {} since inventory is full",
                objects[object_id].name
            ),
            YELLOW,
        );
    } else {
        let item = objects.swap_remove(object_id);
        game.messages
            .add(format!("You picked up {}", item.name), YELLOW);
        let index = game.inventory.len();
        let slot = item.equipment.map(|e| e.slot);
        game.inventory.push(item);

        if let Some(slot) = slot {
            if get_equipped_in_slot(slot, &game.inventory).is_none() {
                game.inventory[index].equip(&mut game.messages);
            }
        }
    }
}

fn drop_item(inventory_id: usize, game: &mut Game, objects: &mut Vec<Object>) {
    let mut item = game.inventory.remove(inventory_id);
    item.set_pos(objects[PLAYER].x, objects[PLAYER].y);
    game.messages
        .add(format!("You dropped a {}", item.name), YELLOW);
    if item.equipment.is_some() {
        item.dequip(&mut game.messages);
    }
    objects.push(item);
}

fn toggle_equipment(
    inventory_id: usize,
    _tcod: &mut Tcod,
    game: &mut Game,
    _objects: &mut Vec<Object>,
) -> UseResult {
    let equipment = match game.inventory[inventory_id].equipment {
        Some(equipment) => equipment,
        None => return UseResult::Cancelled,
    };
    if equipment.equipped {
        game.inventory[inventory_id].dequip(&mut game.messages);
    } else {
        if let Some(current) = get_equipped_in_slot(equipment.slot, &game.inventory) {
            game.inventory[current].dequip(&mut game.messages);
        }
        game.inventory[inventory_id].equip(&mut game.messages);
    }

    UseResult::Used
}

enum UseResult {
    UsedUp,
    Used,
    Cancelled,
}

fn use_item(
    inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
) -> PlayerAction {
    use Item::*;

    if let Some(item) = game.inventory[inventory_id].item {
        let on_use = match item {
            Heal { .. } => cast_heal,
            ManaPotion => cast_mana_potion,
            Lightning { .. } => cast_lightning,
            Fireball { .. } => cast_fireball,
            FireballStaff { .. } => cast_fireballstaff,
            Confuse => cast_confuse,
            Sword => toggle_equipment,
            Shield => toggle_equipment,
            Helmet => toggle_equipment,
            BodyArmor => toggle_equipment,
        };
        match on_use(inventory_id, tcod, game, objects) {
            UseResult::UsedUp => {
                game.messages.add(
                    format!("{} is used up", game.inventory[inventory_id].name),
                    YELLOW,
                );
                game.inventory.remove(inventory_id);
                PlayerAction::TookTurn
            }
            UseResult::Cancelled => {
                game.messages.add("Cancelled", WHITE);
                PlayerAction::DidntTakeTurn
            }
            UseResult::Used => PlayerAction::TookTurn,
        }
    } else {
        game.messages.add(
            format!("the {} cannot be used", game.inventory[inventory_id].name),
            WHITE,
        );
        PlayerAction::DidntTakeTurn
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Fighter {
    base_max_hp: i32,
    hp: i32,
    base_defense: i32,
    base_power: i32,
    mana: i32,
    base_max_mana: i32,
    base_spellcast_modifier: i32,
    xp: i32,
    on_death: DeathCallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum DeathCallback {
    Player,
    Monster,
    MonsterArcher,
}

impl DeathCallback {
    fn callback(self, object: &mut Object, game: &mut Game) {
        use DeathCallback::*;
        let callback: fn(&mut Object, &mut Game) = match self {
            Player => player_death,
            Monster => monster_death,
            MonsterArcher => archer_death,
        };
        callback(object, game);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Ai {
    Basic,
    Confused {
        previous_ai: Box<Ai>,
        num_turns: i32,
    },
    Archer {
        range: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PlayerAction {
    TookTurn,
    DidntTakeTurn,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum Item {
    Heal { amount: i32 },
    Lightning { damage: i32 },
    Fireball { charges: i32, damage: i32 },
    Confuse,
    Sword,
    Shield,
    Helmet,
    BodyArmor,
    ManaPotion,
    FireballStaff { range: i32 },
}

fn menu<T: AsRef<str>>(header: &str, options: &[T], width: i32, tcod: &mut Tcod) -> Option<usize> {
    assert!(
        options.len() <= 26,
        "Cannot have a menu with more than 26 options."
    );

    // calculate the total height of the header (after auto-wrap) and one line per option
    let header_height = if header.is_empty() {
        0
    } else {
        tcod.root
            .get_height_rect(0, 0, width, SCREEN_HEIGHT, header)
    };
    let height = options.len() as i32 + header_height;

    // create an offscreen console that represents the menu's window
    let mut window = Offscreen::new(width, height);

    // print the header, with autowrap
    window.set_default_foreground(WHITE);
    window.print_rect_ex(
        0,
        0,
        width,
        height,
        BackgroundFlag::None,
        TextAlignment::Left,
        header,
    );

    for (index, option_text) in options.iter().enumerate() {
        let menu_letter = (b'a' + index as u8) as char;
        let text = format!("({}) {}", menu_letter, option_text.as_ref());
        window.print_ex(
            0,
            header_height + index as i32,
            BackgroundFlag::None,
            TextAlignment::Left,
            text,
        );
    }

    let x = SCREEN_WIDTH / 2 - width / 2;
    let y = SCREEN_HEIGHT / 2 - height / 2;
    blit(
        &window,
        (0, 0),
        (width, height),
        &mut tcod.root,
        (x, y),
        1.0,
        0.7,
    );

    tcod.root.flush();

    tcod.key = Default::default();

    while match tcod::input::check_for_event(input::KEY_PRESS) {
        Some((
            _,
            Event::Key(Key {
                code: input::KeyCode::Text,
                printable: c,
                ..
            }),
        )) => {
            if c.is_alphabetic() {
                let index = c.to_ascii_lowercase() as usize - 'a' as usize;
                if index < options.len() {
                    return Some(index);
                }
            }
            false
        }
        Some((
            _,
            Event::Key(Key {
                code: input::KeyCode::Escape,
                ..
            }),
        )) => false,
        _ => true,
    } {}

    None

    //while key.is_none() {
    //key = match tcod::input::check_for_event(input::KEY_PRESS) {
    //Some((_, Event::Key(k))) => Some(k),
    //_ => None,
    //};
    //}

    //let key = tcod.root.wait_for_keypress(true);

    //match input::check_for_event(input::MOUSE | input::KEY_PRESS) {
    //Some((_, Event::Mouse(m))) => tcod.mouse = m,
    //Some((_, Event::Key(k))) => tcod.key = k,
    //_ => tcod.key = Default::default(),
    //}

    //tcod.key = Default::default();
    //if let Some(key) = key {
    //k
}

fn inventory_menu(inventory: &Vec<Object>, header: &str, tcod: &mut Tcod) -> Option<usize> {
    let options = if inventory.len() == 0 {
        vec!["Inventory is empty".into()]
    } else {
        inventory
            .iter()
            .map(|item| match item.equipment {
                Some(equipment) if equipment.equipped => {
                    format!("{} (on {})", item.name, equipment.slot)
                }
                _ => item.name.clone(),
            })
            .collect()
    };

    let inventory_index = menu(header, &options, INVENTORY_WIDTH, tcod);

    if inventory.len() > 0 {
        inventory_index
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Equipment {
    slot: Slot,
    equipped: bool,
    power_bonus: i32,
    defense_bonus: i32,
    max_hp_bonus: i32,
    max_mana_bonus: i32,
    spellcast_bonus: i32,
    mana_usage: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum Slot {
    LeftHand,
    RightHand,
    Head,
    Body,
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            Slot::LeftHand => write!(f, "left hand"),
            Slot::RightHand => write!(f, "right hand"),
            Slot::Head => write!(f, "head"),
            Slot::Body => write!(f, "body"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Object {
    x: i32,
    y: i32,
    char: char,
    color: Color,
    name: String,
    blocks: bool,
    alive: bool,
    always_visible: bool,
    level: i32,
    fighter: Option<Fighter>,
    ai: Option<Ai>,
    item: Option<Item>,
    equipment: Option<Equipment>,
}

impl Object {
    pub fn new(x: i32, y: i32, char: char, name: &str, color: Color, blocks: bool) -> Self {
        Object {
            x,
            y,
            char,
            color,
            name: name.into(),
            blocks,
            level: 0,
            always_visible: false,
            alive: false,
            fighter: None,
            ai: None,
            item: None,
            equipment: None,
        }
    }

    pub fn take_damage(&mut self, damage: i32, game: &mut Game) -> Option<i32> {
        if let Some(fighter) = self.fighter.as_mut() {
            if damage > 0 {
                fighter.hp -= damage;
            }
        }
        if let Some(fighter) = self.fighter {
            if fighter.hp <= 0 {
                self.alive = false;
                fighter.on_death.callback(self, game);
                return Some(fighter.xp);
            }
        }
        None
    }

    pub fn get_all_equipped(&self, game: &Game) -> Vec<Equipment> {
        if self.name == "player" {
            game.inventory
                .iter()
                .filter(|item| item.equipment.map_or(false, |e| e.equipped))
                .map(|item| item.equipment.unwrap())
                .collect()
        } else {
            vec![]
        }
    }

    pub fn heal(&mut self, amount: i32, game: &Game) {
        let max_hp = self.max_hp(game);
        if let Some(ref mut fighter) = self.fighter {
            fighter.hp = if fighter.hp + amount > max_hp {
                max_hp
            } else {
                fighter.hp + amount
            };
        }
    }

    pub fn regenerate_mana(&mut self, amount: i32, game: &Game) {
        let max_mana = self.max_mana(game);
        if let Some(ref mut fighter) = self.fighter {
            fighter.mana = if fighter.mana + amount > max_mana {
                max_mana
            } else {
                fighter.mana + amount
            };
        }
    }

    pub fn max_hp(&self, game: &Game) -> i32 {
        let base_max_hp = self.fighter.map_or(0, |f| f.base_max_hp);
        let bonus: i32 = self
            .get_all_equipped(game)
            .iter()
            .map(|e| e.max_hp_bonus)
            .sum();

        base_max_hp + bonus
    }

    pub fn max_mana(&self, game: &Game) -> i32 {
        let base_max_mana = self.fighter.map_or(0, |f| f.base_max_mana);
        let bonus: i32 = self
            .get_all_equipped(game)
            .iter()
            .map(|e| e.max_mana_bonus)
            .sum();

        base_max_mana + bonus
    }

    pub fn spellcast_only(&self, game: &Game) -> i32 {
        let mut mana = self.fighter.map_or(0, |f| f.mana);
        let base_spellcast = self.fighter.map_or(0, |f| f.base_spellcast_modifier);
        let mut bonus = 0;

        for e in self.get_all_equipped(game) {
            if e.mana_usage <= mana {
                mana -= e.mana_usage;
                bonus += e.spellcast_bonus;
            }
        }

        base_spellcast + bonus
    }

    pub fn spellcast(&mut self, game: &Game) -> i32 {
        let mut mana = self.fighter.map_or(0, |f| f.mana);
        let base_spellcast = self.fighter.map_or(0, |f| f.base_spellcast_modifier);
        let mut bonus = 0;

        for e in self.get_all_equipped(game) {
            if e.mana_usage <= mana {
                mana -= e.mana_usage;
                bonus += e.spellcast_bonus;
            }
        }

        self.fighter.as_mut().map(|f| f.mana = mana);

        base_spellcast + bonus
    }

    pub fn power(&self, game: &Game) -> i32 {
        let base_power = self.fighter.map_or(0, |f| f.base_power);
        let bonus: i32 = self
            .get_all_equipped(game)
            .iter()
            .map(|e| e.power_bonus)
            .sum();

        base_power + bonus
    }

    pub fn defense(&self, game: &Game) -> i32 {
        let base_defense = self.fighter.map_or(0, |f| f.base_defense);
        let bonus: i32 = self
            .get_all_equipped(game)
            .iter()
            .map(|e| e.defense_bonus)
            .sum();

        base_defense + bonus
    }

    pub fn equip(&mut self, messages: &mut Messages) {
        if self.item.is_none() {
            messages.add(
                format!("Can't equip {:?} because it is not an Item", self),
                WHITE,
            );
            return;
        }

        if let Some(ref mut equipment) = self.equipment {
            if !equipment.equipped {
                equipment.equipped = true;
                messages.add(
                    format!("Equipped {} on {}", self.name, equipment.slot),
                    YELLOW,
                )
            }
        } else {
            messages.add(
                format!("Cant equip {:?} because it is not an Equipment", self),
                WHITE,
            );
        }
    }

    pub fn dequip(&mut self, messages: &mut Messages) {
        if self.item.is_none() {
            messages.add(
                format!("Can't dequip {:?} because it is not an Item", self),
                WHITE,
            );
            return;
        }
        if let Some(ref mut equipment) = self.equipment {
            if equipment.equipped {
                equipment.equipped = false;
                messages.add(
                    format!("Dequipped {} from slot {}", self.name, equipment.slot),
                    YELLOW,
                );
            }
        } else {
            messages.add(
                format!("Cant deequip {:?} because it is not an Equipment", self),
                WHITE,
            );
        }
    }

    pub fn level_up(tcod: &mut Tcod, _game: &mut Game, objects: &mut [Object]) {
        let player = &mut objects[PLAYER];
        let level_up_xp = LEVEL_UP_BASE + player.level * LEVEL_UP_FACTOR;
        let fighter = player.fighter.as_mut().unwrap();
        if fighter.xp >= level_up_xp {
            let mut choice = None;
            while choice.is_none() {
                choice = menu(
                    "Level up! Choose a stat to raise:\n",
                    &[
                        format!("Constitution (+20 to HP, from {})", fighter.base_max_hp),
                        format!("Strength (+1 to attack, from {})", fighter.base_power),
                        format!("Agility (+1 to defense, from {})", fighter.base_defense),
                        format!("Arcane knowlage (+1 to spellcasting, from {})", fighter.base_spellcast_modifier),
                        format!("Arcane power (+20 to mana, from {})", fighter.base_max_mana),
                    ],
                    LEVEL_SCREEN_WIDTH,
                    tcod,
                )
            }

            fighter.xp -= level_up_xp;
            player.level += 1;
            match choice.unwrap() {
                0 => {
                    fighter.base_max_hp += 20;
                    fighter.hp += 20;
                }
                1 => {
                    fighter.base_power += 1;
                }
                2 => {
                    fighter.base_defense += 1;
                }
                3 => {
                    fighter.base_spellcast_modifier += 1;
                }
                4 => {
                    fighter.base_max_mana += 20;
                    fighter.mana += 20;
                }
                _ => unreachable!(),
            }
        }
    }

    pub fn get_xp(&mut self, xp: i32) {
        self.fighter.as_mut().unwrap().xp += xp;
    }

    pub fn attack(&mut self, target: &mut Object, game: &mut Game, object: Option<usize>) {
        let mut msg = String::new();

        let damage = calculate_damage(
            if let Some(id) = object {
                if let Object {
                    item: Some(item),
                    name,
                    ..
                } = &game.inventory[id]
                {
                    match item {
                        Item::Lightning { damage } => {
                            msg = String::from("with lightning");
                            *damage
                        }
                        Item::Fireball { damage, .. } => {
                            msg = String::from("with fireball");
                            *damage
                        }
                        Item::FireballStaff { .. } => {
                            msg = String::from("with fireball staff");
                            self.spellcast(game)
                        }
                        Item::Heal { .. }
                        | Item::Confuse { .. }
                        | Item::Shield
                        | Item::Helmet
                        | Item::Sword
                        | Item::ManaPotion
                        | Item::BodyArmor => {
                            msg = format!("since {} is not a weapon", name);
                            0
                        }
                    }
                } else {
                    0
                }
            } else {
                self.power(game)
            } as f32,
            target.defense(game) as f32,
        );

        if damage > 0 {
            game.messages.add(
                format!(
                    "{} attacks {} for {} hp {}",
                    self.name, target.name, damage, msg
                ),
                LIGHT_RED,
            );
            if let Some(xp) = target.take_damage(damage, game) {
                self.get_xp(xp);
            }
        } else {
            game.messages.add(
                format!(
                    "{} attacks {} but it has no effect {}",
                    self.name, target.name, msg
                ),
                LIGHT_GREEN,
            );
        }
    }

    pub fn move_by(id: usize, dx: i32, dy: i32, map: &Map, objects: &mut Vec<Object>) {
        let (x, y) = objects[id].pos();
        if !is_blocked(x + dx, y + dy, map, objects) && objects[id].alive {
            objects[id].set_pos(x + dx, y + dy);
        }
    }

    pub fn player_move_or_attack(dx: i32, dy: i32, game: &mut Game, objects: &mut Vec<Object>) {
        let x = objects[PLAYER].x + dx;
        let y = objects[PLAYER].y + dy;

        let target_id = objects
            .iter()
            .position(|object| object.fighter.is_some() && object.pos() == (x, y));

        match target_id {
            Some(target_id) => {
                let (target, player) = mut_two(target_id, PLAYER, objects);
                player.attack(target, game, None);
            }
            None => {
                Object::move_by(PLAYER, dx, dy, &game.map, objects);
            }
        }
    }

    pub fn draw(&self, con: &mut dyn Console) {
        con.set_default_foreground(self.color);
        con.put_char(self.x, self.y, self.char, BackgroundFlag::None);
    }

    pub fn set_pos(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    pub fn pos(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn distance_to(&self, other: &Object) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        ((dx.pow(2) + dy.pow(2)) as f32).sqrt()
    }

    pub fn distance_to_point(&self, (x, y): (i32, i32)) -> f32 {
        let dx = self.x - x;
        let dy = self.y - y;
        ((dx.pow(2) + dy.pow(2)) as f32).sqrt()
    }
}

fn player_death(player: &mut Object, game: &mut Game) {
    game.messages.add("You died!", DARK_RED);

    player.char = '%';
    player.color = DARK_RED;
}

fn archer_death(archer: &mut Object, game: &mut Game) {
    game.messages.add(
        format!(
            "{} is dead! You gain {} experience points",
            archer.name,
            archer.fighter.unwrap().xp
        ),
        DARK_GREEN,
    );
    archer.char = '%';
    archer.color = DARK_RED;
    archer.blocks = false;
    archer.fighter = None;
    archer.ai = None;
    archer.name = format!("remains of {}", archer.name);
}

fn monster_death(monster: &mut Object, game: &mut Game) {
    game.messages.add(
        format!(
            "{} is dead! You gain {} experience points",
            monster.name,
            monster.fighter.unwrap().xp
        ),
        DARK_GREEN,
    );
    monster.char = '%';
    monster.color = DARK_RED;
    monster.blocks = false;
    monster.fighter = None;
    monster.ai = None;
    monster.name = format!("remains of {}", monster.name);
}

fn get_names_under_mouse(mouse: Mouse, objects: &Vec<Object>, fov_map: &FovMap) -> String {
    let (x, y) = (mouse.cx as i32, mouse.cy as i32);

    let names = objects
        .iter()
        .filter(|obj| obj.pos() == (x, y) && fov_map.is_in_fov(obj.x, obj.y))
        .map(|obj| obj.name.clone())
        .collect::<Vec<_>>();

    names.join(", ")
}

fn move_towards(id: usize, target_x: i32, target_y: i32, map: &Map, objects: &mut Vec<Object>) {
    let dx = target_x - objects[id].x;
    let dy = target_y - objects[id].y;
    let distance = ((dx.pow(2) + dy.pow(2)) as f32).sqrt();

    // normalize
    let dx = (dx as f32 / distance).round() as i32;
    let dy = (dy as f32 / distance).round() as i32;

    Object::move_by(id, dx, dy, map, objects);
}

fn mut_two<T>(first_index: usize, second_index: usize, items: &mut [T]) -> (&mut T, &mut T) {
    assert!(first_index != second_index);
    let split_at_index = cmp::max(first_index, second_index);
    let (first_slice, second_slice) = items.split_at_mut(split_at_index);
    if first_index < second_index {
        (&mut first_slice[first_index], &mut second_slice[0])
    } else {
        (&mut second_slice[0], &mut first_slice[second_index])
    }
}

fn ai_basic(monster_id: usize, tcod: &Tcod, game: &mut Game, objects: &mut Vec<Object>) -> Ai {
    let (monster_x, monster_y) = objects[monster_id].pos();
    if tcod.fov.is_in_fov(monster_x, monster_y) {
        if objects[monster_id].distance_to(&objects[PLAYER]) >= 2.0 {
            let (player_x, player_y) = objects[PLAYER].pos();
            move_towards(monster_id, player_x, player_y, &game.map, objects);
        } else if objects[PLAYER].fighter.map_or(false, |f| f.hp > 0) {
            let (monster, player) = mut_two(monster_id, PLAYER, objects);
            monster.attack(player, game, None);
        }
    }
    Ai::Basic
}

fn ai_confused(
    monster_id: usize,
    _tcod: &Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
    previous_ai: Box<Ai>,
    num_turns: i32,
) -> Ai {
    Object::move_by(
        monster_id,
        rand::thread_rng().gen_range(-1, 2),
        rand::thread_rng().gen_range(-1, 2),
        &game.map,
        objects,
    );

    if num_turns <= 1 {
        game.messages.add(
            format!("the {} is no longer confused", objects[monster_id].name),
            RED,
        );
        *previous_ai
    } else {
        Ai::Confused {
            previous_ai,
            num_turns: num_turns - 1,
        }
    }
}

fn ai_archer(
    monster_id: usize,
    tcod: &Tcod,
    game: &mut Game,
    objects: &mut Vec<Object>,
    range: i32,
) -> Ai {
    let (monster_x, monster_y) = objects[monster_id].pos();
    if tcod.fov.is_in_fov(monster_x, monster_y) {
        if objects[monster_id].distance_to(&objects[PLAYER]) >= range as f32 {
            let (player_x, player_y) = objects[PLAYER].pos();
            move_towards(monster_id, player_x, player_y, &game.map, objects);
        } else if objects[PLAYER].fighter.map_or(false, |f| f.hp > 0) {
            let (monster, player) = mut_two(monster_id, PLAYER, objects);
            monster.attack(player, game, None);
        }
    }
    Ai::Archer { range }
}

fn ai_take_turn(monster_id: usize, tcod: &Tcod, game: &mut Game, objects: &mut Vec<Object>) {
    use Ai::*;
    if let Some(ai) = objects[monster_id].ai.take() {
        let new_ai = match ai {
            Basic => ai_basic(monster_id, tcod, game, objects),
            Archer { range } => ai_archer(monster_id, tcod, game, objects, range),
            Confused {
                previous_ai,
                num_turns,
            } => ai_confused(monster_id, tcod, game, objects, previous_ai, num_turns),
        };
        objects[monster_id].ai = Some(new_ai);
    }
}

fn is_blocked(x: i32, y: i32, map: &Map, objects: &Vec<Object>) -> bool {
    map[x as usize][y as usize].blocked
        || objects
            .iter()
            .any(|object| object.blocks && object.pos() == (x, y))
}

fn handle_keys(tcod: &mut Tcod, game: &mut Game, objects: &mut Vec<Object>) -> PlayerAction {
    use tcod::input::Key;
    use tcod::input::KeyCode::*;

    use PlayerAction::*;

    let player_alive = objects[PLAYER].alive;

    match (tcod.key, tcod.key.text(), player_alive) {
        (
            Key {
                code: Enter,
                alt: true,
                ..
            },
            _,
            _,
        ) => {
            let fullscreen = tcod.root.is_fullscreen();
            tcod.root.set_fullscreen(!fullscreen);
            DidntTakeTurn
        }
        (Key { code: Text, .. }, "c", true) => {
            let player = &objects[PLAYER];
            let level = player.level;
            let level_up_xp = LEVEL_UP_BASE + player.level * LEVEL_UP_FACTOR;
            if let Some(fighter) = player.fighter.as_ref() {
                let msg = format!(
                    "Character information

Level: {}
Experience: {}
Experience to level up: {}

Maximum HP: {}
Attack: {}
Maximum mana: {}
Spellcasting: {}
Defense: {}",
                    level,
                    fighter.xp,
                    level_up_xp,
                    player.max_hp(game),
                    player.power(game),
                    player.max_mana(game),
                    player.spellcast_only(game),
                    player.defense(game)
                );
                msgbox(&msg, CHARACTER_SCREEN_WIDTH, tcod);
            }

            DidntTakeTurn
        }
        (Key { code: Text, .. }, "<", true) => {
            let player_on_stairs = objects
                .iter()
                .any(|object| object.pos() == objects[PLAYER].pos() && object.name == "stairs");
            if player_on_stairs {
                next_level(tcod, game, objects);
            }
            DidntTakeTurn
        }
        (Key { code: Text, .. }, "u", true) => {
            let mut in_right_hand: Vec<(usize, &Object)> = game
                .inventory
                .iter()
                .enumerate()
                .filter(|(_id, o)| {
                    o.item.is_some()
                        && match o.equipment {
                            Some(e) => e.slot == Slot::RightHand && e.equipped,
                            None => false,
                        }
                })
                .collect();
            if in_right_hand.len() > 1 {
                println!("More that one item in right hand?");
                return DidntTakeTurn;
            }
            if let Some((inventory_id, _object)) = in_right_hand.pop() {
                use_item(inventory_id, tcod, game, objects);
                TookTurn
            } else {
                game.messages
                    .add("You do not hold anything in your right hand.", YELLOW);
                DidntTakeTurn
            }
        }
        (Key { code: Text, .. }, "d", true) => {
            let inventory_index = inventory_menu(&game.inventory, "Select an item to drop\n", tcod);
            if let Some(inventory_index) = inventory_index {
                drop_item(inventory_index, game, objects);
            }
            DidntTakeTurn
        }
        (Key { code: Text, .. }, "g", true) => {
            let item_id = objects
                .iter()
                .position(|object| object.pos() == objects[PLAYER].pos() && object.item.is_some());
            if let Some(item_id) = item_id {
                pick_item_up(item_id, game, objects);
            }
            DidntTakeTurn
        }
        (Key { code: Text, .. }, "i", true) => {
            let inventory_index = inventory_menu(
                &game.inventory,
                "Press the key next to an item to use itm or any other to cancel.\n",
                tcod,
            );
            if let Some(inventory_index) = inventory_index {
                use_item(inventory_index, tcod, game, objects)
            } else {
                DidntTakeTurn
            }
        }
        (Key { code: Text, .. }, "k", true) => {
            Object::player_move_or_attack(0, -1, game, objects);
            TookTurn
        }
        (Key { code: Text, .. }, "j", true) => {
            Object::player_move_or_attack(0, 1, game, objects);
            TookTurn
        }
        (Key { code: Text, .. }, "h", true) => {
            Object::player_move_or_attack(-1, 0, game, objects);
            TookTurn
        }
        (Key { code: Text, .. }, "l", true) => {
            Object::player_move_or_attack(1, 0, game, objects);
            TookTurn
        }
        (Key { code: Text, .. }, "s", true) => {
            game.messages.add("You chose to sleep thos round", YELLOW);
            TookTurn
        }
        (Key { code: Escape, .. }, _, _) => Exit,
        _ => DidntTakeTurn,
    }
}

fn msgbox(text: &str, width: i32, tcod: &mut Tcod) {
    let options: &[&str] = &[];
    menu(text, options, width, tcod);
}

fn new_game(tcod: &mut Tcod) -> (Game, Vec<Object>) {
    let mut player = Object::new(0, 0, '@', "player", WHITE, true);
    player.alive = true;
    player.fighter = Some(Fighter {
        base_max_hp: 100,
        hp: 100,
        base_defense: 1,
        base_power: 2,
        xp: 0,
        base_max_mana: 20,
        base_spellcast_modifier: 3,
        mana: 20,
        on_death: DeathCallback::Player,
    });

    //let npc = Object::new(
    //SCREEN_WIDTH / 2 - 5,
    //SCREEN_HEIGHT / 2,
    //'@',
    //"NPC",
    //YELLOW,
    //false,
    //);

    let mut objects = vec![player];

    let mut game = Game {
        map: make_map(&mut objects, 0),
        messages: Messages::new(),
        inventory: vec![],
        dungeon_level: 0,
    };

    let mut dagger = Object::new(0, 0, '-', "dagger", SKY, false);
    dagger.item = Some(Item::Sword);
    dagger.equipment = Some(Equipment {
        equipped: true,
        slot: Slot::LeftHand,
        max_hp_bonus: 0,
        defense_bonus: 0,
        power_bonus: 2,
        mana_usage: 0,
        max_mana_bonus: 0,
        spellcast_bonus: 0,
    });
    game.inventory.push(dagger);

    initialize_fov(tcod, &game.map);

    game.messages.add("player has spawned in a dungeon", YELLOW);

    (game, objects)
}

fn initialize_fov(tcod: &mut Tcod, map: &Map) {
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            tcod.fov.set(
                x,
                y,
                !map[x as usize][y as usize].block_sight,
                !map[x as usize][y as usize].blocked,
            );
        }
    }

    // unexplored areas start black
    tcod.con.clear();
}

fn play_game(tcod: &mut Tcod, game: &mut Game, objects: &mut Vec<Object>) {
    let mut previous_player_pos = (-1, -1);

    while !tcod.root.window_closed() {
        tcod.con.clear();

        match input::check_for_event(input::MOUSE | input::KEY_PRESS) {
            Some((_, Event::Mouse(m))) => tcod.mouse = m,
            Some((_, Event::Key(k))) => tcod.key = k,
            _ => tcod.key = Default::default(),
        }

        let fov_recomute = previous_player_pos != (objects[PLAYER].pos());
        render_all(tcod, game, objects, fov_recomute);

        tcod.root.flush();

        Object::level_up(tcod, game, objects);

        previous_player_pos = objects[PLAYER].pos();

        let player_action = handle_keys(tcod, game, objects);
        if player_action == PlayerAction::Exit {
            save_game(game, objects).unwrap();
            break;
        }

        if objects[PLAYER].alive && player_action != PlayerAction::DidntTakeTurn {
            for id in 0..objects.len() {
                if objects[id].ai.is_some() {
                    ai_take_turn(id, tcod, game, objects);
                }
            }
        }
    }
}

fn next_level(tcod: &mut Tcod, game: &mut Game, objects: &mut Vec<Object>) {
    game.messages.add(
        "You take a moment to rest, and recover your strength.",
        VIOLET,
    );
    let heal_hp = objects[PLAYER].max_hp(game) / 2;
    objects[PLAYER].heal(heal_hp, game);

    game.messages.add(
        "After a rare moment of peace, you decent deeper into the heart of the dungeon...",
        YELLOW,
    );
    game.dungeon_level += 1;
    game.map = make_map(objects, game.dungeon_level);
    initialize_fov(tcod, &game.map);
}

fn main_menu(tcod: &mut Tcod) {
    let img = tcod::image::Image::from_file("images/menu_background.png")
        .ok()
        .expect("Background image not found!");

    while !tcod.root.window_closed() {
        tcod::image::blit_2x(&img, (0, 0), (-1, -1), &mut tcod.root, (0, 0));

        tcod.root.set_default_foreground(LIGHT_YELLOW);
        tcod.root.print_ex(
            SCREEN_WIDTH / 2,
            SCREEN_HEIGHT / 2 - 4,
            BackgroundFlag::None,
            TextAlignment::Center,
            "Some random roguelike",
        );
        tcod.root.print_ex(
            SCREEN_WIDTH / 2,
            SCREEN_HEIGHT - 2,
            BackgroundFlag::None,
            TextAlignment::Center,
            "by some random dev",
        );

        let choices = &["Play new game", "Continue last game", "Quit"];
        let choice = menu("", choices, 24, tcod);

        match choice {
            Some(0) => {
                // new game
                let (mut game, mut objects) = new_game(tcod);
                play_game(tcod, &mut game, &mut objects);
            }
            Some(1) => {
                match load_game() {
                    Ok((mut game, mut objects)) => {
                        initialize_fov(tcod, &game.map);
                        play_game(tcod, &mut game, &mut objects);
                    }
                    Err(_e) => {
                        msgbox("\nNo saved game to load.\n", 24, tcod);
                        continue;
                    }
                };
            }
            Some(2) => {
                break;
            }
            _ => {}
        }
    }
}

fn save_game(game: &Game, objects: &[Object]) -> Result<(), Box<dyn Error>> {
    let save_data = serde_json::to_string(&(game, objects))?;
    let mut file = File::create("gamesave.json")?;
    file.write_all(save_data.as_bytes())?;
    Ok(())
}

fn load_game() -> Result<(Game, Vec<Object>), Box<dyn Error>> {
    let mut json_save_state = String::new();
    let mut file = File::open("gamesave.json")?;
    file.read_to_string(&mut json_save_state)?;
    let result = serde_json::from_str::<(Game, Vec<Object>)>(&json_save_state)?;
    Ok(result)
}

fn main() {
    let root = Root::initializer()
        .font("fonts/arial10x10.png", FontLayout::Tcod)
        .font_type(FontType::Greyscale)
        .size(SCREEN_WIDTH, SCREEN_HEIGHT)
        .title("libtcod tutorial")
        .init();

    let mut tcod = Tcod {
        root,
        con: Offscreen::new(MAP_WIDTH, MAP_HEIGHT),
        panel: Offscreen::new(SCREEN_WIDTH, PANEL_HEIGHT),
        fov: FovMap::new(MAP_WIDTH, MAP_HEIGHT),
        key: Default::default(),
        mouse: Default::default(),
    };

    tcod::system::set_fps(LIMIT_FPS);

    main_menu(&mut tcod);
}
