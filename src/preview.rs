//! Load only the assets for the selected preview; never silently replace the beauty.
use crate::config::{Config, PreviewMode};
use anyhow::{Context, Result};

pub enum PreviewScene {
    Beauty(image::RgbaImage),
    Arm(Box<crate::arm_assets::ArmPoseAssets>),
}
pub struct PreviewAsset {
    pub variant: crate::config::SceneVariant,
    pub scene: PreviewScene,
    pub sleeve: Option<crate::sleeve_assets::SleeveAssets>,
    pub record_lighting: Option<crate::sleeve_assets::SleeveAssets>,
    pub vinyl_detail: Option<crate::vinyl_assets::VinylAssets>,
    pub buttons: Option<crate::button_assets::ButtonAssets>,
    pub knob: Option<crate::knob_assets::KnobAssets>,
    pub empty_stand: Option<crate::stand_assets::StandAssets>,
    pub stack_sleeve: Option<crate::sleeve_assets::SleeveAssets>,
}
impl PreviewAsset {
    pub fn load(config: &Config) -> Result<Self> {
        use crate::asset_validation::SceneBudget;
        let package = config.scene.selected_package()?;
        let mut budget = SceneBudget::default();
        let scene = match config.scene.mode {
            PreviewMode::Beauty => {
                let image = crate::asset_validation::load_rgba(&package.background, &mut budget)
                    .with_context(|| {
                        format!(
                            "Cannot load Blender beauty {}",
                            package.background.display()
                        )
                    })?;
                tracing::info!(path=%package.background.display(), variant=?config.scene.variant, width=image.width(), height=image.height(), "Loaded complete static Blender beauty");
                PreviewScene::Beauty(image)
            }
            PreviewMode::Arm => {
                let mut assets = crate::arm_assets::ArmPoseAssets::load_budgeted(
                    &package.background,
                    &package.arm_poses,
                    &mut budget,
                )?;
                if config.render.arm_interpolation {
                    assets.flow = Some(crate::arm_flow::ArmFlowAssets::load_budgeted(
                        &package.arm_flow,
                        &package.arm_poses,
                        &assets.manifest,
                        true,
                        &mut budget,
                    )?);
                } else if package.arm_flow.exists() {
                    assets.flow = budget.optional(
                        |budget| {
                            crate::arm_flow::ArmFlowAssets::load_budgeted(
                                &package.arm_flow,
                                &package.arm_poses,
                                &assets.manifest,
                                false,
                                budget,
                            )
                        },
                        "artwork foreground masks",
                    )?;
                }
                // Bad optional boundaries fall back, but oversized files reject the package.
                budget.optional(
                    |_| assets.load_update_crop(&package.arm_poses, &package.arm_flow),
                    "arm update boundary",
                )?;
                PreviewScene::Arm(Box::new(assets))
            }
        };
        let dimensions = match &scene {
            PreviewScene::Beauty(image) => image.dimensions(),
            PreviewScene::Arm(assets) => assets.base.dimensions(),
        };
        let mut paper = |path: Option<&std::path::PathBuf>, kind: &str| -> Result<_> {
            match path {
                Some(path) => budget.optional(
                    |budget| {
                        crate::sleeve_assets::SleeveAssets::load_budgeted(
                            path,
                            &package.background,
                            dimensions,
                            kind,
                            budget,
                        )
                    },
                    kind,
                ),
                None => Ok(None),
            }
        };
        let sleeve = paper(
            package
                .sleeve
                .as_ref()
                .filter(|_| config.artwork.sleeve_enabled),
            "sleeve",
        )?;
        let record_lighting = paper(
            package
                .record_lighting
                .as_ref()
                .filter(|_| config.artwork.record_lighting),
            "record_lighting",
        )?;
        let vinyl_detail = match package
            .vinyl_detail
            .as_ref()
            .filter(|_| config.scene.mode == PreviewMode::Arm)
        {
            Some(path) => budget.optional(
                |budget| {
                    crate::vinyl_assets::VinylAssets::load_budgeted(
                        path,
                        &package.background,
                        dimensions,
                        budget,
                    )
                },
                "vinyl detail",
            )?,
            None => None,
        };
        let buttons = match package.buttons.as_ref() {
            Some(path) => budget.optional(
                |budget| {
                    crate::button_assets::ButtonAssets::load_budgeted(
                        path,
                        &package.background,
                        dimensions,
                        budget,
                    )
                },
                "button animation",
            )?,
            None => None,
        };
        let root = package
            .background
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let knob_path = root.join("knob/atlas.json");
        let knob = if config.scene.mode == PreviewMode::Arm && knob_path.exists() {
            budget.optional(
                |budget| {
                    crate::knob_assets::KnobAssets::load_budgeted(
                        &knob_path,
                        &package.background,
                        dimensions,
                        budget,
                    )
                },
                "knob animation",
            )?
        } else {
            None
        };
        let stand_path = root.join("empty-stand/atlas.json");
        let empty_stand = if stand_path.exists() {
            budget.optional(
                |budget| {
                    crate::stand_assets::StandAssets::load_budgeted(
                        &stand_path,
                        &package.background,
                        dimensions,
                        budget,
                    )
                },
                "empty stand",
            )?
        } else {
            None
        };
        let stack_path = root.join("stack-sleeve/atlas.json");
        let stack_sleeve = if stack_path.exists() {
            budget.optional(
                |budget| {
                    crate::sleeve_assets::SleeveAssets::load_budgeted(
                        &stack_path,
                        &package.background,
                        dimensions,
                        "stack",
                        budget,
                    )
                },
                "previous-track sleeve",
            )?
        } else {
            None
        };
        tracing::info!(
            retained_mib = budget.bytes() as f64 / 1048576.0,
            limit_mib = crate::asset_validation::MAX_SCENE_BYTES / 1048576,
            "Scene asset memory budget accepted"
        );
        Ok(Self {
            variant: config.scene.variant,
            scene,
            sleeve,
            record_lighting,
            vinyl_detail,
            buttons,
            knob,
            empty_stand,
            stack_sleeve,
        })
    }
}
#[cfg(test)]
mod tests;
