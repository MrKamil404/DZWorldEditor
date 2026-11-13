//! A new mission: the files a server needs, with the settings the game starts from, and the
//! way an existing mission becomes the mission of another map.
use super::xml::{self, Element, Node, XmlDoc};
use super::{AreaFlags, FileData, JsonDoc, Mission, TextDoc};
use crate::world::WorldMeta;

/// The folder name of the mission of a map.
pub fn mission_name(meta: &WorldMeta) -> String {
    format!("dayzOffline.{}", meta.map_id.to_ascii_lowercase())
}

pub const INIT_C: &str = r#"void main()
{
	//INIT ECONOMY--------------------------------------
	Hive ce = CreateHive();
	if ( ce )
		ce.InitOffline();

	//DATE RESET AFTER ECONOMY INIT-------------------------
	int year, month, day, hour, minute;
	int reset_month = 9, reset_day = 20;
	GetGame().GetWorld().GetDate(year, month, day, hour, minute);

	if ((month == reset_month) && (day < reset_day))
	{
		GetGame().GetWorld().SetDate(year, reset_month, reset_day, hour, minute);
	}
	else if ((month == reset_month + 1) && (day > reset_day))
	{
		GetGame().GetWorld().SetDate(year, reset_month, reset_day, hour, minute);
	}
	else if ((month < reset_month) || (month > reset_month + 1))
	{
		GetGame().GetWorld().SetDate(year, reset_month, reset_day, hour, minute);
	}
}

class CustomMission: MissionServer
{
	void SetRandomHealth(EntityAI itemEnt)
	{
		if ( itemEnt )
		{
			float rndHlt = Math.RandomFloat( 0.45, 0.65 );
			itemEnt.SetHealth01( "", "", rndHlt );
		}
	}

	override PlayerBase CreateCharacter(PlayerIdentity identity, vector pos, ParamsReadContext ctx, string characterName)
	{
		Entity playerEnt;
		playerEnt = GetGame().CreatePlayer( identity, characterName, pos, 0, "NONE" );
		Class.CastTo( m_player, playerEnt );

		GetGame().SelectPlayer( identity, m_player );

		return m_player;
	}

	override void StartingEquipSetup(PlayerBase player, bool clothesChosen)
	{
		EntityAI itemClothing;
		EntityAI itemEnt;

		itemClothing = player.FindAttachmentBySlotName( "Body" );
		if ( itemClothing )
		{
			SetRandomHealth( itemClothing );

			itemEnt = itemClothing.GetInventory().CreateInInventory( "BandageDressing" );
			player.SetQuickBarEntityShortcut(itemEnt, 2);

			string chemlightArray[] = { "Chemlight_White", "Chemlight_Yellow", "Chemlight_Green", "Chemlight_Red" };
			int rndIndex = Math.RandomInt( 0, 4 );
			itemEnt = itemClothing.GetInventory().CreateInInventory( chemlightArray[rndIndex] );
			SetRandomHealth( itemEnt );
			player.SetQuickBarEntityShortcut(itemEnt, 1);
		}

		itemClothing = player.FindAttachmentBySlotName( "Legs" );
		if ( itemClothing )
			SetRandomHealth( itemClothing );
	}
};

Mission CreateCustomMission(string path)
{
	return new CustomMission();
}
"#;

const GLOBALS: [(&str, u8, &str); 30] = [
    ("AnimalMaxCount", 0, "200"),
    ("CleanupAvoidance", 0, "100"),
    ("CleanupLifetimeDeadAnimal", 0, "1200"),
    ("CleanupLifetimeDeadInfected", 0, "330"),
    ("CleanupLifetimeDeadPlayer", 0, "3600"),
    ("CleanupLifetimeDefault", 0, "45"),
    ("CleanupLifetimeLimit", 0, "50"),
    ("CleanupLifetimeRuined", 0, "330"),
    ("FlagRefreshFrequency", 0, "432000"),
    ("FlagRefreshMaxDuration", 0, "3456000"),
    ("FoodDecay", 0, "1"),
    ("IdleModeCountdown", 0, "60"),
    ("IdleModeStartup", 0, "1"),
    ("InitialSpawn", 0, "100"),
    ("LootDamageMax", 1, "0.82"),
    ("LootDamageMin", 1, "0.0"),
    ("LootProxyPlacement", 0, "1"),
    ("LootSpawnAvoidance", 0, "100"),
    ("RespawnAttempt", 0, "2"),
    ("RespawnLimit", 0, "20"),
    ("RespawnTypes", 0, "12"),
    ("RestartSpawn", 0, "0"),
    ("SpawnInitial", 0, "1200"),
    ("TimeHopping", 0, "60"),
    ("TimeLogin", 0, "15"),
    ("TimeLogout", 0, "15"),
    ("TimePenalty", 0, "20"),
    ("WorldWetTempUpdate", 0, "1"),
    ("ZombieMaxCount", 0, "1000"),
    ("ZoneSpawnDist", 0, "300"),
];

pub const CATEGORIES: [&str; 8] = ["tools", "containers", "clothes", "lootdispatch", "food", "weapons", "books", "explosives"];
pub const TAGS: [&str; 3] = ["floor", "shelves", "ground"];
pub const USAGES: [&str; 17] = ["Military", "Police", "Medic", "Firefighter", "Industrial", "Farm", "Coast", "Town", "Village", "Hunting", "Office", "School", "Prison", "Lunapark", "SeasonalEvent", "ContaminatedArea", "Historical"];
pub const VALUES: [&str; 5] = ["Tier1", "Tier2", "Tier3", "Tier4", "Unique"];

const ROOTCLASSES: [(&str, &str, bool); 10] = [
    ("DefaultWeapon", "", false),
    ("DefaultMagazine", "", false),
    ("Inventory_Base", "", false),
    ("HouseNoDestruct", "", true),
    ("SurvivorBase", "character", true),
    ("DZ_LightAI", "character", true),
    ("CarScript", "car", true),
    ("BoatScript", "car", true),
    ("MotorbikeScript", "car", true),
    ("HouseDestructible", "", true),
];

const CE_DEFAULTS: [(&str, &str); 18] = [
    ("dyn_radius", "30"),
    ("dyn_smin", "0"),
    ("dyn_smax", "0"),
    ("dyn_dmin", "1"),
    ("dyn_dmax", "5"),
    ("log_ce_loop", "false"),
    ("log_ce_dynamicevent", "false"),
    ("log_ce_vehicle", "false"),
    ("log_ce_lootspawn", "false"),
    ("log_ce_lootcleanup", "false"),
    ("log_ce_lootrespawn", "false"),
    ("log_ce_statistics", "false"),
    ("log_ce_zombie", "false"),
    ("log_storageinfo", "false"),
    ("log_hivewarning", "true"),
    ("log_missionfilewarning", "true"),
    ("save_events_startup", "true"),
    ("save_types_startup", "true"),
];

/// (section, init, load, respawn, save) of `economy.xml`.
const ECONOMY: [(&str, u8, u8, u8, u8); 8] = [
    ("dynamic", 1, 1, 1, 1),
    ("animals", 1, 0, 1, 0),
    ("zombies", 1, 0, 1, 0),
    ("vehicles", 1, 1, 1, 1),
    ("randoms", 0, 0, 1, 0),
    ("custom", 0, 0, 0, 0),
    ("building", 1, 1, 0, 1),
    ("player", 1, 1, 1, 1),
];

/// (element, current actual/time/duration, limits, timelimits, changelimits, thresholds).
type WeatherPart = (&'static str, [&'static str; 3], [&'static str; 2], [&'static str; 2], [&'static str; 2], Option<[&'static str; 3]>);
const WEATHER: [WeatherPart; 6] = [
    ("overcast", ["0.45", "120", "240"], ["0.0", "1.0"], ["600", "900"], ["0.0", "1.0"], None),
    ("fog", ["0.05", "120", "240"], ["0.02", "0.08"], ["900", "900"], ["0.0", "1.0"], None),
    ("rain", ["0.0", "60", "120"], ["0.0", "1.0"], ["60", "120"], ["0.0", "1.0"], Some(["0.6", "1.0", "60"])),
    ("windMagnitude", ["8.0", "120", "240"], ["0.0", "20.0"], ["120", "240"], ["0.0", "20.0"], None),
    ("windDirection", ["0.0", "120", "240"], ["-3.14", "3.14"], ["60", "120"], ["-1.0", "1.0"], None),
    ("snowfall", ["0.0", "0", "32768"], ["0.0", "0.0"], ["300", "3600"], ["0.0", "0.0"], Some(["1.0", "1.0", "120"])),
];

pub fn weather_doc() -> XmlDoc {
    let mut root = Element::new("weather").with_attr("reset", "0").with_attr("enable", "1");
    for (name, cur, lim, tl, cl, th) in WEATHER {
        let mut e = Element::new(name)
            .with_child(Element::new("current").with_attr("actual", cur[0]).with_attr("time", cur[1]).with_attr("duration", cur[2]))
            .with_child(Element::new("limits").with_attr("min", lim[0]).with_attr("max", lim[1]))
            .with_child(Element::new("timelimits").with_attr("min", tl[0]).with_attr("max", tl[1]))
            .with_child(Element::new("changelimits").with_attr("min", cl[0]).with_attr("max", cl[1]));
        if let Some(t) = th {
            e.push(Element::new("thresholds").with_attr("min", t[0]).with_attr("max", t[1]).with_attr("end", t[2]));
        }
        root.push(e);
    }
    root.push(Element::new("storm").with_attr("density", "1.0").with_attr("threshold", "0.9").with_attr("timeout", "45"));
    doc(root)
}

fn doc(root: Element) -> XmlDoc {
    let mut d = XmlDoc::new(root);
    d.style.space_before_close = true;
    d
}

fn named(tag: &str, names: &[&str]) -> Vec<Element> {
    names.iter().map(|n| Element::new(tag).with_attr("name", *n)).collect()
}

fn list(tag: &str, children: Vec<Element>) -> Element {
    let mut e = Element::new(tag);
    for c in children {
        e.push(c);
    }
    e
}

/// The parameters of a section of `cfgplayerspawnpoints.xml`, as the game ships them.
pub fn spawn_section(name: &str, group: Option<(&str, [f32; 2])>) -> Element {
    let p = |tag: &str, v: &str| Element::new(tag).with_text(v);
    let (lmin, lmax) = if name == "fresh" { ("30", "70") } else { ("25", "70") };
    let mut sec = Element::new(name)
        .with_child(list("spawn_params", vec![p("min_dist_infected", lmin), p("max_dist_infected", lmax), p("min_dist_player", "65"), p("max_dist_player", "150"), p("min_dist_static", "0"), p("max_dist_static", "2")]))
        .with_child(list("generator_params", vec![p("grid_density", "4"), p("grid_width", "200"), p("grid_height", "200"), p("min_dist_static", "0"), p("max_dist_static", "2"), p("min_steepness", "-45"), p("max_steepness", "45")]))
        .with_child(list("group_params", vec![p("enablegroups", "true"), p("groups_as_regular", "true"), p("lifetime", "240"), p("counter", "-1")]));
    let mut bubbles = Element::new("generator_posbubbles");
    if let Some((g, [x, z])) = group {
        bubbles.push(Element::new("group").with_attr("name", g).with_child(Element::new("pos").with_attr("x", super::gen::coord(x)).with_attr("z", super::gen::coord(z))));
    }
    sec.push(bubbles);
    sec
}

pub fn gameplay_json() -> serde_json::Value {
    serde_json::json!({
        "version": 123,
        "GeneralData": {
            "disableBaseDamage": false,
            "disableContainerDamage": false,
            "disableRespawnDialog": false,
            "disableRespawnInUnconsciousness": false
        },
        "PlayerData": {
            "disablePersonalLight": false,
            "StaminaData": {
                "sprintStaminaModifierErc": 1.0, "sprintStaminaModifierCro": 1.0, "staminaWeightLimitThreshold": 6000.0, "staminaMax": 100.0,
                "staminaKgToStaminaPercentPenalty": 1.75, "staminaMinCap": 5.0, "sprintSwimmingStaminaModifier": 1.0, "sprintLadderStaminaModifier": 1.0,
                "meleeStaminaModifier": 1.0, "obstacleTraversalStaminaModifier": 1.0, "holdBreathStaminaModifier": 1.0
            },
            "ShockHandlingData": { "shockRefillSpeedConscious": 5.0, "shockRefillSpeedUnconscious": 1.0, "allowRefillSpeedModifier": true },
            "MovementData": { "timeToStrafeJog": 0.1, "rotationSpeedJog": 0.15, "timeToSprint": 0.45, "timeToStrafeSprint": 0.3, "rotationSpeedSprint": 0.15, "allowStaminaAffectInertia": true },
            "DrowningData": { "staminaDepletionSpeed": 10.0, "healthDepletionSpeed": 10.0, "shockDepletionSpeed": 10.0 },
            "WeaponObstructionData": { "staticMode": 1, "dynamicMode": 1 }
        },
        "WorldsData": {
            "lightingConfig": 0,
            "objectSpawnersArr": [],
            "environmentMinTemps": [-3, -2, 0, 4, 9, 14, 18, 17, 13, 11, 9, 0],
            "environmentMaxTemps": [3, 5, 7, 14, 19, 24, 26, 25, 18, 14, 10, 5],
            "wetnessWeightModifiers": [1.0, 1.0, 1.33, 1.66, 2.0]
        },
        "BaseBuildingData": {
            "HologramData": {
                "disableIsCollidingBBoxCheck": false, "disableIsCollidingPlayerCheck": false, "disableIsClippingRoofCheck": false, "disableIsBaseViableCheck": false,
                "disableIsCollidingGPlotCheck": false, "disableIsCollidingAngleCheck": false, "disableIsPlacementPermittedCheck": false, "disableHeightPlacementCheck": false,
                "disableIsUnderwaterCheck": false, "disableIsInTerrainCheck": false, "disableColdAreaBuildingCheck": true,
                "disallowedTypesInUnderground": ["FenceKit", "TerritoryFlagKit", "WatchtowerKit"]
            },
            "ConstructionData": {
                "disablePerformRoofCheck": false, "disableIsCollidingCheck": false, "disableDistanceCheck": false,
                "disableItemFalling": false, "disableDropAttachmentsOnFall": false, "disableRefundMaterialsOnFall": false
            }
        },
        "UIData": {
            "use3DMap": false,
            "HitIndicationData": {
                "hitDirectionOverrideEnabled": false, "hitDirectionBehaviour": 1, "hitDirectionStyle": 0, "hitDirectionIndicatorColorStr": "0xffbb0a1e",
                "hitDirectionMaxDuration": 2.0, "hitDirectionBreakPointRelative": 0.2, "hitDirectionScatter": 10.0, "hitIndicationPostProcessEnabled": true
            }
        },
        "MapData": { "ignoreMapOwnership": false, "ignoreNavItemsOwnership": false, "displayPlayerPosition": false, "displayNavInfo": true },
        "VehicleData": { "boatDecayMultiplier": 1, "canDetachAttachedCarWheels": true, "canDamageAttachedCarWheels": true, "canDetachAttachedBikeWheels": true, "canDamageAttachedBikeWheels": true }
    })
}

/// Cells of a new `areaflags.map` per side.
pub const AREAFLAGS_CELLS: u32 = 4096;

/// A blank area-flag map over the world.
pub fn blank_areaflags(meta: &WorldMeta, usages: usize, values: usize) -> AreaFlags {
    let world = meta.size_m().round().max(1.0) as u32;
    AreaFlags::new(AREAFLAGS_CELLS, world, super::areaflags::bits_for(usages.max(17)).max(32), super::areaflags::bits_for(values.max(5)))
}

/// A new mission for the map, with every file the server reads.
pub fn new_mission(meta: &WorldMeta) -> Mission {
    let mut m = Mission { name: mission_name(meta), ..Default::default() };
    let x = |root: Element| FileData::Xml(doc(root));
    m.put("init.c", FileData::Text(TextDoc::new(INIT_C)));
    m.put("db/types.xml", x(Element::new("types")));
    m.put("db/events.xml", x(Element::new("events")));
    m.put("db/messages.xml", x(Element::new("messages")));
    let mut globals = Element::new("variables");
    for (n, t, v) in GLOBALS {
        globals.push(Element::new("var").with_attr("name", n).with_attr("type", t.to_string()).with_attr("value", v));
    }
    m.put("db/globals.xml", x(globals));
    let mut eco = Element::new("economy");
    for (n, i, l, r, s) in ECONOMY {
        eco.push(Element::new(n).with_attr("init", i.to_string()).with_attr("load", l.to_string()).with_attr("respawn", r.to_string()).with_attr("save", s.to_string()));
    }
    m.put("db/economy.xml", x(eco));
    let mut classes = Element::new("classes");
    for (n, act, no_lod) in ROOTCLASSES {
        let mut e = Element::new("rootclass").with_attr("name", n);
        if !act.is_empty() {
            e = e.with_attr("act", act);
        }
        if no_lod {
            e = e.with_attr("reportMemoryLOD", "no");
        }
        classes.push(e);
    }
    let mut defaults = Element::new("defaults");
    for (n, v) in CE_DEFAULTS {
        defaults.push(Element::new("default").with_attr("name", n).with_attr("value", v));
    }
    m.put("cfgeconomycore.xml", x(Element::new("economycore").with_child(classes).with_child(defaults)));
    m.put(
        "cfglimitsdefinition.xml",
        x(Element::new("lists").with_child(list("categories", named("category", &CATEGORIES))).with_child(list("tags", named("tag", &TAGS))).with_child(list("usageflags", named("usage", &USAGES))).with_child(list("valueflags", named("value", &VALUES)))),
    );
    m.put("cfglimitsdefinitionuser.xml", x(Element::new("user_lists").with_child(Element::new("usageflags")).with_child(Element::new("valueflags"))));
    m.put("cfgenvironment.xml", x(Element::new("env").with_child(Element::new("territories"))));
    m.put("cfgeventgroups.xml", x(Element::new("eventgroupdef")));
    m.put("cfgeventspawns.xml", x(Element::new("eventposdef")));
    let spawn = (meta.spawn[0], meta.spawn[1]);
    let group = Some(("Spawn", [spawn.0, spawn.1]));
    m.put("cfgplayerspawnpoints.xml", x(Element::new("playerspawnpoints").with_child(spawn_section("fresh", group)).with_child(spawn_section("hop", group)).with_child(spawn_section("travel", group))));
    m.put("cfgrandompresets.xml", x(Element::new("randompresets")));
    m.put("cfgspawnabletypes.xml", x(Element::new("spawnabletypes")));
    m.put("cfgignorelist.xml", x(Element::new("ignore")));
    m.put("cfgweather.xml", FileData::Xml(weather_doc()));
    m.put("cfggameplay.json", FileData::Json(JsonDoc::new(gameplay_json())));
    m.put("cfgeffectarea.json", FileData::Json(JsonDoc::new(serde_json::json!({ "Areas": [], "SafePositions": [] }))));
    m.put("cfgundergroundtriggers.json", FileData::Json(JsonDoc::new(serde_json::json!({ "Triggers": [] }))));
    let mut proto = Element::new("prototype");
    proto.push(Element::new("defaults").with_child(Element::new("default").with_attr("name", "group").with_attr("lootmax", "6")).with_child(Element::new("default").with_attr("name", "container").with_attr("lootmax", "4")).with_child(Element::new("default").with_attr("name", "keepInvalidPoints").with_attr("enabled", "yes")));
    m.put("mapgroupproto.xml", x(proto));
    m.put("mapgrouppos.xml", x(Element::new("map")));
    m.put("mapclusterproto.xml", x(Element::new("prototype").with_child(Element::new("clusters"))));
    m.put("mapgroupdirt.xml", x(Element::new("map")));
    m.put("areaflags.map", FileData::AreaFlags(blank_areaflags(meta, USAGES.len(), VALUES.len())));
    m.removed.clear();
    m
}

/// Makes a mission of another map the mission of this map: everything placed on the other
/// map goes (spawns, zones, areas, map groups, area flags, persistence), the economy stays.
pub fn adapt(mut m: Mission, meta: &WorldMeta) -> Mission {
    m.name = mission_name(meta);
    let strip = |m: &mut Mission, rel: &str, f: &dyn Fn(&mut Element)| {
        if let Some(d) = m.xml_mut(rel) {
            f(&mut d.root);
            m.touch(rel);
        }
    };
    for f in m.ce_files("eventspawns") {
        strip(&mut m, &f, &|r| {
            for ev in r.children_named_mut("event") {
                ev.remove_children("pos");
            }
        });
    }
    strip(&mut m, "cfgplayerspawnpoints.xml", &|r| {
        for sec in r.elements_mut() {
            if let Some(b) = sec.child_mut("generator_posbubbles") {
                b.children.clear();
                b.push(Element::new("group").with_attr("name", "Spawn").with_child(Element::new("pos").with_attr("x", super::gen::coord(meta.spawn[0])).with_attr("z", super::gen::coord(meta.spawn[1]))));
            }
        }
    });
    for f in m.territory_files() {
        strip(&mut m, &f, &|r| {
            for t in r.children_named_mut("territory") {
                t.remove_children("zone");
            }
        });
    }
    strip(&mut m, "mapgrouppos.xml", &|r| {
        r.retain_elements(|e| e.name != "group" && e.name != "include");
    });
    let clusters: Vec<String> = m.files.keys().filter(|k| k.to_ascii_lowercase().starts_with("mapgroupcluster")).cloned().collect();
    for k in clusters {
        m.remove(&k);
    }
    if let Some(d) = m.json_mut("cfgeffectarea.json") {
        if let Some(o) = d.value.as_object_mut() {
            for k in ["Areas", "SafePositions"] {
                if o.contains_key(k) {
                    o.insert(k.into(), serde_json::Value::Array(Vec::new()));
                }
            }
        }
        m.touch("cfgeffectarea.json");
    }
    if let Some(d) = m.json_mut("cfgundergroundtriggers.json") {
        if let Some(o) = d.value.as_object_mut() {
            o.insert("Triggers".into(), serde_json::Value::Array(Vec::new()));
        }
        m.touch("cfgundergroundtriggers.json");
    }
    let usages = m.limits("usageflags").len();
    let values = m.limits("valueflags").len();
    m.put("areaflags.map", FileData::AreaFlags(blank_areaflags(meta, usages, values)));
    m.wipe_storage();
    m
}

/// Whether a node is a comment, for tests.
pub fn is_comment(n: &Node) -> bool {
    matches!(n, Node::Comment(_))
}

/// Parses a template text, for tests and tools.
pub fn parse(text: &str) -> XmlDoc {
    xml::parse_str(text).expect("template parses")
}
