use std::{collections::BTreeMap, fs};

use crate::{
    config::load_project_config,
    destination::PlacementStyle,
    scan::{
        apply_placement, apply_placement_with_aliases, scan_project_files,
        scan_project_files_with_aliases, scan_tsconfig,
    },
    source::discover_typescript_resolution_files,
    transaction::write_file_transaction,
};

use super::{
    CompatibilityReexportQuoteStyle, plan_extraction_rewrite, plan_extraction_rewrite_with_aliases,
};

#[test]
fn moves_a_scanned_type_in_a_temporary_typescript_project() {
    let fixture = tempfile::tempdir().expect("creates fixture project");
    let root = fixture.path();
    fs::create_dir_all(root.join("src/features/order")).expect("creates source directory");
    fs::create_dir_all(root.join("src/pages")).expect("creates referrer directory");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true},"include":["src/**/*.ts"]}"#,
    )
    .expect("writes tsconfig");
    fs::write(
        root.join("src/features/order/schema.ts"),
        "export type OrderSummary = { id: string };\nexport const defaultOrder: OrderSummary = { id: 'order-1' };\n",
    )
    .expect("writes declaration source");
    fs::write(
        root.join("src/pages/order-page.ts"),
        "import type { OrderSummary } from '../features/order/schema';\nexport const selected: OrderSummary = { id: 'order-2' };\n",
    )
    .expect("writes referrer source");

    let candidates = apply_placement(
        scan_tsconfig(&root.join("tsconfig.json")).expect("scans fixture project"),
        PlacementStyle::Collocation,
    );
    let candidate = candidates
        .into_iter()
        .find(|candidate| candidate.symbol_name == "OrderSummary")
        .expect("finds shared type candidate");
    assert_eq!(candidate.source_path, "src/features/order/schema.ts");
    assert_eq!(
        candidate.destination_path,
        "src/features/order/types/order.ts"
    );

    let sources = discover_typescript_resolution_files(root, true)
        .expect("discovers rewrite sources")
        .into_iter()
        .map(|path| {
            let source_path = path
                .strip_prefix(root)
                .expect("source is in fixture")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(path).expect("reads rewrite source");
            (source_path, source)
        })
        .collect::<BTreeMap<_, _>>();
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans extraction");

    write_file_transaction(root, &changes).expect("commits extraction");

    assert_eq!(
        fs::read_to_string(root.join("src/features/order/types/order.ts"))
            .expect("reads destination"),
        "export type OrderSummary = { id: string };\n"
    );
    let source = fs::read_to_string(root.join("src/features/order/schema.ts"))
        .expect("reads rewritten declaration source");
    assert!(source.contains("import type { OrderSummary } from './types/order';"));
    assert!(source.contains("export type { OrderSummary } from './types/order';"));
    assert!(source.contains("export const defaultOrder: OrderSummary"));
    assert_eq!(
        fs::read_to_string(root.join("src/pages/order-page.ts")).expect("reads rewritten referrer"),
        "import type { OrderSummary } from '../features/order/types/order';\nexport const selected: OrderSummary = { id: 'order-2' };\n"
    );
}

#[test]
fn retargets_a_barrel_in_a_temporary_typescript_project() {
    let fixture = tempfile::tempdir().expect("creates fixture project");
    let root = fixture.path();
    fs::create_dir_all(root.join("src/features/order")).expect("creates source directory");
    fs::create_dir_all(root.join("src/pages")).expect("creates referrer directory");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true},"include":["src/**/*.ts"]}"#,
    )
    .expect("writes tsconfig");
    fs::write(
        root.join("src/features/order/schema.ts"),
        "export type OrderSummary = { id: string };\n",
    )
    .expect("writes declaration source");
    fs::write(
        root.join("src/features/order/index.ts"),
        "export type { OrderSummary } from './schema';\n",
    )
    .expect("writes barrel source");
    let page = "import type { OrderSummary } from '../features/order';\nexport const selected: OrderSummary = { id: 'order-2' };\n";
    fs::write(root.join("src/pages/order-page.ts"), page).expect("writes referrer source");

    let candidates = apply_placement(
        scan_tsconfig(&root.join("tsconfig.json")).expect("scans fixture project"),
        PlacementStyle::Collocation,
    );
    let candidate = candidates
        .into_iter()
        .find(|candidate| candidate.symbol_name == "OrderSummary")
        .expect("finds shared type candidate");
    assert_eq!(
        candidate.destination_index_path.as_deref(),
        Some("src/features/order/index.ts")
    );

    let sources = discover_typescript_resolution_files(root, true)
        .expect("discovers rewrite sources")
        .into_iter()
        .map(|path| {
            let source_path = path
                .strip_prefix(root)
                .expect("source is in fixture")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(path).expect("reads rewrite source");
            (source_path, source)
        })
        .collect::<BTreeMap<_, _>>();
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans extraction");

    write_file_transaction(root, &changes).expect("commits extraction");

    assert_eq!(
        fs::read_to_string(root.join("src/features/order/types/order.ts"))
            .expect("reads destination"),
        "export type OrderSummary = { id: string };\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("src/features/order/index.ts")).expect("reads barrel"),
        "export type { OrderSummary } from './types/order';\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("src/pages/order-page.ts")).expect("reads referrer"),
        page
    );
}

#[test]
fn moves_local_type_dependencies_in_a_temporary_typescript_project() {
    let fixture = tempfile::tempdir().expect("creates fixture project");
    let root = fixture.path();
    fs::create_dir_all(root.join("src/features/order")).expect("creates source directory");
    fs::create_dir_all(root.join("src/pages")).expect("creates referrer directory");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true},"include":["src/**/*.ts"]}"#,
    )
    .expect("writes tsconfig");
    fs::write(
        root.join("src/features/order/schema.ts"),
        "type Money = { cents: number };\nexport type OrderSummary = { total: Money };\n",
    )
    .expect("writes declaration source");
    fs::write(
        root.join("src/pages/order-page.ts"),
        "import type { OrderSummary } from '../features/order/schema';\nexport const selected: OrderSummary = { total: { cents: 2000 } };\n",
    )
    .expect("writes referrer source");

    let candidates = apply_placement(
        scan_tsconfig(&root.join("tsconfig.json")).expect("scans fixture project"),
        PlacementStyle::Collocation,
    );
    let candidate = candidates
        .into_iter()
        .find(|candidate| candidate.symbol_name == "OrderSummary")
        .expect("finds shared type candidate");
    assert_eq!(candidate.local_dependency_names, ["Money"]);

    let sources = discover_typescript_resolution_files(root, true)
        .expect("discovers rewrite sources")
        .into_iter()
        .map(|path| {
            let source_path = path
                .strip_prefix(root)
                .expect("source is in fixture")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(path).expect("reads rewrite source");
            (source_path, source)
        })
        .collect::<BTreeMap<_, _>>();
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans extraction");

    write_file_transaction(root, &changes).expect("commits extraction");

    let destination = fs::read_to_string(root.join("src/features/order/types/order.ts"))
        .expect("reads destination");
    assert!(destination.contains("type Money = { cents: number };"));
    assert!(destination.contains("export type OrderSummary = { total: Money };"));
    assert_eq!(
        fs::read_to_string(root.join("src/features/order/schema.ts")).expect("reads source"),
        "export type { OrderSummary } from './types/order';\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("src/pages/order-page.ts")).expect("reads referrer"),
        "import type { OrderSummary } from '../features/order/types/order';\nexport const selected: OrderSummary = { total: { cents: 2000 } };\n"
    );
}

#[test]
fn preserves_path_aliases_in_a_temporary_typescript_project() {
    let fixture = tempfile::tempdir().expect("creates fixture project");
    let root = fixture.path();
    fs::create_dir_all(root.join("src/features/order")).expect("creates source directory");
    fs::create_dir_all(root.join("src/pages")).expect("creates referrer directory");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]},"strict":true},"include":["src/**/*.ts"]}"#,
    )
    .expect("writes tsconfig");
    fs::write(
        root.join("src/features/order/schema.ts"),
        "export type OrderSummary = { id: string };\n",
    )
    .expect("writes declaration source");
    fs::write(
        root.join("src/pages/order-page.ts"),
        "import type { OrderSummary } from '@/features/order/schema';\nexport const selected: OrderSummary = { id: 'order-2' };\n",
    )
    .expect("writes referrer source");

    let config = load_project_config(&root.join("tsconfig.json")).expect("loads config");
    let candidates = apply_placement_with_aliases(
        scan_tsconfig(&root.join("tsconfig.json")).expect("scans fixture project"),
        PlacementStyle::Collocation,
        config.base_url.as_deref(),
        &config.paths,
    );
    let candidate = candidates
        .into_iter()
        .find(|candidate| candidate.symbol_name == "OrderSummary")
        .expect("finds shared type candidate");

    let sources = discover_typescript_resolution_files(root, true)
        .expect("discovers rewrite sources")
        .into_iter()
        .map(|path| {
            let source_path = path
                .strip_prefix(root)
                .expect("source is in fixture")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(path).expect("reads rewrite source");
            (source_path, source)
        })
        .collect::<BTreeMap<_, _>>();
    let changes = plan_extraction_rewrite_with_aliases(
        &[candidate],
        &sources,
        config.base_url.as_deref(),
        &config.paths,
        CompatibilityReexportQuoteStyle::Apply,
    )
    .expect("plans extraction");

    write_file_transaction(root, &changes).expect("commits extraction");

    assert_eq!(
        fs::read_to_string(root.join("src/pages/order-page.ts")).expect("reads referrer"),
        "import type { OrderSummary } from '@/features/order/types/order';\nexport const selected: OrderSummary = { id: 'order-2' };\n"
    );
}

#[test]
fn preserves_configured_aliases_without_requiring_base_url() {
    for (config, inherited) in [
        (
            r#"{"compilerOptions":{"strict":true,"noEmit":true,"paths":{"~/*":["./app/*"],"@/*":["./app/*"]}},"include":["app/**/*.ts"]}"#,
            false,
        ),
        (
            r#"{"extends":"./configs/base.json","include":["app/**/*.ts"]}"#,
            true,
        ),
        (
            r#"{"compilerOptions":{"strict":true,"noEmit":true,"baseUrl":"app","ignoreDeprecations":"6.0","paths":{"~/*":["*"],"@/*":["./*"]}},"include":["app/**/*.ts"]}"#,
            false,
        ),
    ] {
        let fixture = tempfile::tempdir().expect("creates fixture");
        let root = fixture.path();
        fs::create_dir_all(root.join("app/hooks/cart")).unwrap();
        fs::create_dir_all(root.join("app/components/detail")).unwrap();
        fs::create_dir_all(root.join("app/feature")).unwrap();
        fs::write(root.join("tsconfig.json"), config).unwrap();
        if inherited {
            fs::create_dir_all(root.join("configs")).unwrap();
            fs::write(root.join("configs/base.json"), r#"{"compilerOptions":{"strict":true,"noEmit":true,"paths":{"~/*":["../app/*"],"@/*":["../app/*"]}}}"#).unwrap();
        }
        fs::write(
            root.join("app/hooks/cart/use-cart.ts"),
            "export type Cart = { id: string };\nexport type CartExtra = { count: number };\nexport const marker = 1;\n",
        )
        .unwrap();
        fs::write(
            root.join("app/feature/index.ts"),
            "export type { Cart, CartExtra } from '~/hooks/cart/use-cart';\n",
        )
        .unwrap();
        fs::write(root.join("app/components/detail/view.ts"), "import type { Cart, CartExtra } from '~/hooks/cart/use-cart';\nexport const view: Cart = { id: 'x' };\nexport const extra: CartExtra = { count: 1 };\n").unwrap();
        fs::write(root.join("app/components/detail/other.ts"), "import type { Cart } from '@/hooks/cart/use-cart';\nexport const other: Cart = { id: 'x' };\n").unwrap();
        fs::write(root.join("app/components/detail/both-mixed.ts"), "import { type Cart as One, marker } from '~/hooks/cart/use-cart';\nimport type { Cart as Two } from '@/hooks/cart/use-cart';\nexport type Both = [One, Two];\nexport const value = marker;\n").unwrap();
        fs::write(root.join("app/components/detail/both-multiple.ts"), "import type { Cart as TildeCart, CartExtra as TildeExtra } from '~/hooks/cart/use-cart';\nimport type { Cart as AtCart, CartExtra as AtExtra } from '@/hooks/cart/use-cart';\nexport type All = [TildeCart, TildeExtra, AtCart, AtExtra];\n").unwrap();
        fs::write(root.join("app/components/detail/both.ts"), "import type { Cart as RelCart } from '../../hooks/cart/use-cart';\nimport type { Cart as TildeCart, CartExtra } from '~/hooks/cart/use-cart';\nimport type { Cart as AtCart } from '@/hooks/cart/use-cart';\nexport const one: TildeCart = { id: 'x' };\nexport const two: AtCart = { id: 'x' };\nexport const three: CartExtra = { count: 1 };\nexport const four: RelCart = { id: 'x' };\nexport type Inline = import('~/hooks/cart/use-cart').Cart;\n").unwrap();
        fs::write(root.join("app/components/detail/relative.ts"), "import type { Cart } from '../../hooks/cart/use-cart';\nexport const relative: Cart = { id: 'x' };\n").unwrap();
        fs::write(root.join("app/components/detail/barrel.ts"), "import type { Cart } from '../../feature';\nexport const barrel: Cart = { id: 'x' };\n").unwrap();
        let cfg = load_project_config(&root.join("tsconfig.json")).unwrap();
        let candidates = scan_tsconfig(&root.join("tsconfig.json"))
            .unwrap()
            .into_iter()
            .filter(|c| c.symbol_name == "Cart" || c.symbol_name == "CartExtra")
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 2);
        let sources = discover_typescript_resolution_files(root, true)
            .unwrap()
            .into_iter()
            .map(|p| {
                (
                    p.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read_to_string(p).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let changes = plan_extraction_rewrite_with_aliases(
            &candidates,
            &sources,
            cfg.base_url.as_deref(),
            &cfg.explicit_paths,
            CompatibilityReexportQuoteStyle::Apply,
        )
        .unwrap();
        write_file_transaction(root, &changes).unwrap();
        assert!(
            fs::read_to_string(root.join("app/components/detail/view.ts"))
                .unwrap()
                .contains("from '~/hooks/cart/types/cart'")
        );
        assert!(
            fs::read_to_string(root.join("app/components/detail/other.ts"))
                .unwrap()
                .contains("from '@/hooks/cart/types/cart'")
        );
        let both = fs::read_to_string(root.join("app/components/detail/both.ts")).unwrap();
        assert!(both.contains("from '~/hooks/cart/types/cart'"), "{both}");
        assert!(both.contains("from '@/hooks/cart/types/cart'"), "{both}");
        assert!(
            both.contains("from '../../hooks/cart/types/cart'"),
            "{both}"
        );
        let both_multiple =
            fs::read_to_string(root.join("app/components/detail/both-multiple.ts")).unwrap();
        assert!(
            both_multiple.contains("from '~/hooks/cart/types/cart'"),
            "{both_multiple}"
        );
        assert!(
            both_multiple.contains("from '@/hooks/cart/types/cart'"),
            "{both_multiple}"
        );
        let both_mixed =
            fs::read_to_string(root.join("app/components/detail/both-mixed.ts")).unwrap();
        assert!(
            both_mixed.contains("from '~/hooks/cart/types/cart'"),
            "{both_mixed}"
        );
        assert!(
            both_mixed.contains("from '@/hooks/cart/types/cart'"),
            "{both_mixed}"
        );
        assert!(
            fs::read_to_string(root.join("app/components/detail/relative.ts"))
                .unwrap()
                .contains("from '../../hooks/cart/types/cart'")
        );
        assert!(
            fs::read_to_string(root.join("app/feature/index.ts"))
                .unwrap()
                .contains("from '~/hooks/cart/types/cart'")
        );
        let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../node_modules/typescript/bin/tsc");
        if compiler.exists() {
            let output = std::process::Command::new("node")
                .arg(compiler)
                .arg("--project")
                .arg(root.join("tsconfig.json"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{config}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}

#[test]
fn falls_back_when_an_alias_destination_is_shadowed() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("app/hooks/cart")).unwrap();
    fs::create_dir_all(root.join("app/components")).unwrap();
    fs::create_dir_all(root.join("shadow/hooks/cart/types")).unwrap();
    fs::write(root.join("tsconfig.json"), r#"{"compilerOptions":{"strict":true,"noEmit":true,"paths":{"~/*":["./shadow/*","./app/*"]}},"include":["app/**/*.ts"],"exclude":["shadow"]}"#).unwrap();
    fs::write(
        root.join("app/hooks/cart/use-cart.ts"),
        "export type Cart = { id: string };\n",
    )
    .unwrap();
    fs::write(root.join("app/components/view.ts"), "import type { Cart } from '~/hooks/cart/use-cart';\nexport const view: Cart = { id: 'ok' };\n").unwrap();
    fs::write(
        root.join("shadow/hooks/cart/types/cart.ts"),
        "export type Cart = { wrong: number };\n",
    )
    .unwrap();
    let cfg = load_project_config(&root.join("tsconfig.json")).unwrap();
    let candidate = scan_tsconfig(&root.join("tsconfig.json"))
        .unwrap()
        .into_iter()
        .find(|c| c.symbol_name == "Cart" && c.source_path == "app/hooks/cart/use-cart.ts")
        .unwrap();
    let sources = discover_typescript_resolution_files(root, true)
        .unwrap()
        .into_iter()
        .map(|p| {
            (
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                fs::read_to_string(p).unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let changes = plan_extraction_rewrite_with_aliases(
        &[candidate],
        &sources,
        cfg.base_url.as_deref(),
        &cfg.explicit_paths,
        CompatibilityReexportQuoteStyle::Apply,
    )
    .unwrap();
    write_file_transaction(root, &changes).unwrap();
    assert!(
        fs::read_to_string(root.join("app/components/view.ts"))
            .unwrap()
            .contains("from '../hooks/cart/types/cart'")
    );
    let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/typescript/bin/tsc");
    if compiler.exists() {
        let output = std::process::Command::new("node")
            .arg(compiler)
            .arg("--project")
            .arg(root.join("tsconfig.json"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn uses_a_relative_path_when_destination_leaves_the_alias_mapping() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("app/hooks/cart")).unwrap();
    fs::create_dir_all(root.join("app/components")).unwrap();
    fs::write(root.join("tsconfig.json"), r#"{"compilerOptions":{"strict":true,"noEmit":true,"paths":{"~/*":["./app/*"]}},"include":["app/**/*.ts","shared/**/*.ts"]}"#).unwrap();
    fs::write(
        root.join("app/hooks/cart/use-cart.ts"),
        "export type Cart = { id: string };\n",
    )
    .unwrap();
    fs::write(root.join("app/components/view.ts"), "import type { Cart } from '~/hooks/cart/use-cart';\nexport const view: Cart = { id: 'ok' };\n").unwrap();
    let cfg = load_project_config(&root.join("tsconfig.json")).unwrap();
    let candidate = scan_tsconfig(&root.join("tsconfig.json"))
        .unwrap()
        .into_iter()
        .find(|c| c.symbol_name == "Cart")
        .unwrap();
    let candidate = crate::scan::override_candidate_destination(
        candidate,
        "shared/types/cart.ts".into(),
        cfg.base_url.as_deref(),
        &cfg.explicit_paths,
    );
    let sources = discover_typescript_resolution_files(root, true)
        .unwrap()
        .into_iter()
        .map(|p| {
            (
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                fs::read_to_string(p).unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let changes = plan_extraction_rewrite_with_aliases(
        &[candidate],
        &sources,
        cfg.base_url.as_deref(),
        &cfg.explicit_paths,
        CompatibilityReexportQuoteStyle::Apply,
    )
    .unwrap();
    write_file_transaction(root, &changes).unwrap();
    assert!(
        fs::read_to_string(root.join("app/components/view.ts"))
            .unwrap()
            .contains("from '../../shared/types/cart'")
    );
    let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/typescript/bin/tsc");
    if compiler.exists() {
        let output = std::process::Command::new("node")
            .arg(compiler)
            .arg("--project")
            .arg(root.join("tsconfig.json"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn preserves_configured_alias_and_js_extension_for_nodenext() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("src/feature")).unwrap();
    fs::write(root.join("package.json"), r#"{"type":"module"}"#).unwrap();
    fs::write(root.join("tsconfig.json"), r#"{"compilerOptions":{"strict":true,"noEmit":true,"module":"NodeNext","moduleResolution":"NodeNext","paths":{"@scope/*":["./src/*"]}},"include":["src/**/*.ts"]}"#).unwrap();
    fs::write(
        root.join("src/feature/schema.ts"),
        "export type Item = { id: string };\n",
    )
    .unwrap();
    fs::write(root.join("src/app.ts"), "import type { Item } from '@scope/feature/schema.js';\nexport const item: Item = { id: 'ok' };\n").unwrap();
    let cfg = load_project_config(&root.join("tsconfig.json")).unwrap();
    let candidate = scan_tsconfig(&root.join("tsconfig.json"))
        .unwrap()
        .into_iter()
        .find(|c| c.symbol_name == "Item")
        .unwrap();
    let sources = discover_typescript_resolution_files(root, true)
        .unwrap()
        .into_iter()
        .map(|p| {
            (
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                fs::read_to_string(p).unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let changes = super::plan_extraction_rewrite_with_scoped_resolution_options_and_extension(
        &[candidate],
        &sources,
        cfg.base_url.as_deref(),
        &cfg.explicit_paths,
        &cfg.package_aliases,
        &cfg.root_dirs,
        &cfg.module_suffixes,
        ".js",
        CompatibilityReexportQuoteStyle::Apply,
    )
    .unwrap();
    write_file_transaction(root, &changes).unwrap();
    assert!(
        fs::read_to_string(root.join("src/app.ts"))
            .unwrap()
            .contains("from '@scope/feature/types/feature.js'")
    );
    let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/typescript/bin/tsc");
    if compiler.exists() {
        let output = std::process::Command::new("node")
            .arg(compiler)
            .arg("--project")
            .arg(root.join("tsconfig.json"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn plans_a_relative_import_extraction_without_writing() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(changes["src/shared.ts"].contains("export type { Shared } from './types/src';"));
    assert_eq!(changes["src/types/src.ts"], "export type Shared = {};\n");
    assert!(changes["src/app.ts"].contains("'./types/src'"));
}

#[test]
fn preserves_typescript_module_specifiers() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/javascript.ts".into(),
            "import type { Shared } from './shared.js'; type App = Shared;".into(),
        ),
        (
            "src/typescript.ts".into(),
            "import type { Shared } from './shared.ts'; type App = Shared;".into(),
        ),
        (
            "src/plain.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(changes["src/javascript.ts"].contains("'./types/src.js'"));
    assert!(changes["src/typescript.ts"].contains("'./types/src.ts'"));
    assert!(changes["src/plain.ts"].contains("'./types/src'"));
    assert!(!changes["src/plain.ts"].contains("./types/src.js"));
}

#[test]
fn rewrites_javascript_module_specifiers() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared.js'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(changes["src/app.ts"].contains("'./types/src.js'"));
}

#[test]
fn preserves_imported_type_used_by_moved_and_remaining_function() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/models.ts".into(), "export type Model = { id: string };\n".into()),
        ("src/source.ts".into(), "import type { Model } from './models';\n\nexport type Group = { items: Model[] };\nexport const key = (item: Model): string => item.id;\n".into()),
        ("src/use.ts".into(), "import type { Group } from './source';\nexport type Result = Group;\n".into()),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "Group")
    .unwrap();
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/source.ts"],
        "import type { Model } from './models';\n\nexport const key = (item: Model): string => item.id;\n\nexport type { Group } from './types/src';\n"
    );
    assert_eq!(
        changes["src/types/src.ts"],
        "import type { Model } from '../models';\nexport type Group = { items: Model[] };\n"
    );
    compile_rewrite_fixture(&sources, &changes);
}

fn compile_rewrite_fixture(sources: &BTreeMap<String, String>, changes: &BTreeMap<String, String>) {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"noEmit":true},"include":["src/**/*.ts"]}"#,
    )
    .unwrap();
    for (path, source) in sources {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    write_file_transaction(root, changes).unwrap();
    let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/typescript/bin/tsc");
    if compiler.exists() {
        let output = std::process::Command::new("node")
            .arg(compiler)
            .arg("--project")
            .arg(root.join("tsconfig.json"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn cleans_moved_imports_but_keeps_bindings_used_by_remaining_code() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/models.ts".into(), "export type Model = { id: string };\nexport type Removed = number;\nexport type Remain = boolean;\nexport type Inline = string;\nexport const value = 1;\n".into()),
        ("src/default.ts".into(), "export default interface Default { id: string }\n".into()),
        ("src/namespace.ts".into(), "export type Item = string;\n".into()),
        ("src/source.ts".into(), "import type { Model as LocalModel, Removed, Remain } from './models';\nimport { value, type Inline } from './models';\nimport type Default from './default';\nimport type * as Namespace from './namespace';\n\nexport type Group = [LocalModel, Removed, Inline, Default, Namespace.Item];\nexport function keep(model: LocalModel, remain: Remain, item: Default, ns: Namespace.Item): number { return value; }\n".into()),
        ("src/use.ts".into(), "import type { Group } from './source';\nexport type Result = Group;\n".into()),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "Group")
    .unwrap();
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/source.ts"],
        "import type { Model as LocalModel, Remain } from './models';\nimport { value } from './models';\nimport type Default from './default';\nimport type * as Namespace from './namespace';\n\nexport function keep(model: LocalModel, remain: Remain, item: Default, ns: Namespace.Item): number { return value; }\n\nexport type { Group } from './types/src';\n"
    );
    assert_eq!(
        changes["src/types/src.ts"],
        "import type Default from '../default';\nimport type { Model as LocalModel, Removed, Inline } from '../models';\nimport type * as Namespace from '../namespace';\nexport type Group = [LocalModel, Removed, Inline, Default, Namespace.Item];\n"
    );
    compile_rewrite_fixture(&sources, &changes);
}

#[test]
fn preserves_comments_and_inline_type_import_runtime_semantics() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/models.ts".into(), "export type A = string;\nexport type B = number;\nexport type C = boolean;\n".into()),
        ("src/source.ts".into(), "import type { A /* keep comment */, B } from './models';\nimport { type C } from './models';\n\nexport type Group = [A, C];\nexport const b: B = 1;\n".into()),
        ("src/use.ts".into(), "import type { Group } from './source';\nexport type Result = Group;\n".into()),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "Group")
    .unwrap();
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/source.ts"],
        "import type { A /* keep comment */, B } from './models';\nimport { type C } from './models';\n\nexport const b: B = 1;\n\nexport type { Group } from './types/src';\n"
    );
    assert_eq!(
        changes["src/types/src.ts"],
        "import type { A, C } from '../models';\nexport type Group = [A, C];\n"
    );
    compile_rewrite_fixture(&sources, &changes);
}

#[test]
fn multiple_moved_types_keep_imports_needed_by_copied_private_dependencies() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/models.ts".into(), "export type Model = string;\n".into()),
        ("src/source.ts".into(), "import type { Model } from './models';\n\ntype Private = { item: Model };\nexport type A = Private;\nexport type B = Private;\nexport const keep = (item: Private): Model => item.item;\n".into()),
        ("src/use.ts".into(), "import type { A, B } from './source';\nexport type Result = [A, B];\n".into()),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .filter(|c| c.symbol_name == "A" || c.symbol_name == "B")
    .collect::<Vec<_>>();
    assert_eq!(candidates.len(), 2);
    let changes = plan_extraction_rewrite(&candidates, &sources).unwrap();
    assert_eq!(
        changes["src/source.ts"],
        "import type { Model } from './models';\n\ntype Private = { item: Model };\nexport const keep = (item: Private): Model => item.item;\n\nexport type { A, B } from './types/src';\n"
    );
    assert_eq!(
        changes["src/types/src.ts"],
        "import type { Model } from '../models';\ntype Private = { item: Model };\n\nexport type A = Private;\n\nexport type B = Private;\n"
    );
    compile_rewrite_fixture(&sources, &changes);
}

#[test]
fn places_retargeted_type_import_before_later_sorted_relative_imports() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/utils/z.ts".into(), "export type Shared = string;\n".into()),
        ("src/utils/a.ts".into(), "export const a = 1;\n".into()),
        ("src/pages/view.ts".into(), "import { a } from '../utils/a';\nimport type { Shared } from '../utils/z';\n\nexport const view = (value: Shared) => a + value;\n".into()),
    ]);
    let candidate = shared_candidate(&sources, "Shared");
    let candidate = crate::scan::override_candidate_destination(
        candidate,
        "src/types/z.ts".into(),
        None,
        &BTreeMap::new(),
    );
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/pages/view.ts"],
        "import type { Shared } from '../types/z';\nimport { a } from '../utils/a';\n\nexport const view = (value: Shared) => a + value;\n"
    );
    compile_rewrite_fixture(&sources, &changes);
}

#[test]
fn places_split_type_import_before_earlier_sorted_relative_imports() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/utils/z.ts".into(), "export type Shared = string;\nexport const value = 1;\n".into()),
        ("src/utils/a.ts".into(), "export const a = 1;\n".into()),
        ("src/pages/view.ts".into(), "import { a } from '../utils/a';\nimport { type Shared, value } from '../utils/z';\n\nexport const view = (item: Shared) => a + value + item;\n".into()),
    ]);
    let candidate = shared_candidate(&sources, "Shared");
    let candidate = crate::scan::override_candidate_destination(
        candidate,
        "src/types/z.ts".into(),
        None,
        &BTreeMap::new(),
    );
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/pages/view.ts"],
        "import type { Shared } from '../types/z';\nimport { a } from '../utils/a';\nimport { value } from '../utils/z';\n\nexport const view = (item: Shared) => a + value + item;\n"
    );
    compile_rewrite_fixture(&sources, &changes);
}

#[test]
fn sorted_type_import_relocation_respects_comments_groups_and_crlf() {
    use super::format::earlier_sorted_import_offset;
    let statement = "import type { Shared } from '../types/z';";
    for (source, expected) in [
        (
            "import { a } from '../utils/a';\nimport type { Shared } from '../utils/z';\n",
            true,
        ),
        (
            "import { a } from '../utils/a';\r\nimport type { Shared } from '../utils/z';\r\n",
            true,
        ),
        (
            "import {\n  a,\n} from '../utils/a';\nimport type { Shared } from '../utils/z';\n",
            true,
        ),
        (
            "import { a } from '../utils/a';\n\nimport type { Shared } from '../utils/z';\n",
            false,
        ),
        (
            "// keep with a\nimport { a } from '../utils/a';\nimport type { Shared } from '../utils/z';\n",
            false,
        ),
        (
            "import { a } from '../utils/a'; // keep with a\nimport type { Shared } from '../utils/z';\n",
            false,
        ),
        (
            "import { a } from '../utils/a';\n// keep with Shared\nimport type { Shared } from '../utils/z';\n",
            false,
        ),
        (
            "import { a } from '~/utils/a';\nimport type { Shared } from '../utils/z';\n",
            false,
        ),
        (
            "import { a } from '../utils/zebra';\nimport type { Shared } from '../utils/a';\n",
            false,
        ),
    ] {
        let start = source.find("import type { Shared }").unwrap();
        let end = start + source[start..].find(';').unwrap() + 1;
        assert_eq!(
            earlier_sorted_import_offset(source, start, end, statement).is_some(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn relocated_type_import_retains_crlf_and_unchanged_comments() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/utils/z.ts".into(), "export type Shared = string;\r\n".into()),
        ("src/utils/a.ts".into(), "export const a = 1;\r\n".into()),
        ("src/pages/view.ts".into(), "import { a } from '../utils/a';\r\nimport type { Shared } from '../utils/z';\r\n\r\n// keep body\r\nexport const view = (value: Shared) => a + value;\r\n".into()),
    ]);
    let candidate = shared_candidate(&sources, "Shared");
    let candidate = crate::scan::override_candidate_destination(
        candidate,
        "src/types/z.ts".into(),
        None,
        &BTreeMap::new(),
    );
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/pages/view.ts"],
        "import type { Shared } from '../types/z';\r\nimport { a } from '../utils/a';\r\n\r\n// keep body\r\nexport const view = (value: Shared) => a + value;\r\n"
    );
    compile_rewrite_fixture(&sources, &changes);
}

#[test]
fn preserves_a_source_file_with_value_code() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export type Shared = {}; export const label: Shared = {};".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(changes["src/shared.ts"].contains("export const label: Shared = {};"));
    assert!(changes["src/shared.ts"].contains("import type { Shared } from './types/src';"));
    assert!(changes["src/shared.ts"].contains("export type { Shared } from './types/src';"));
    assert_eq!(
        changes["src/shared.ts"],
        "import type { Shared } from './types/src';\n\nexport const label: Shared = {};\n\nexport type { Shared } from './types/src';\n"
    );
}

#[test]
fn places_remaining_type_imports_with_existing_imports() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/helper.ts".into(),
            "export function helper(value: string) { return value; }".into(),
        ),
        (
            "src/button.tsx".into(),
            "import { helper } from './helper';\nexport interface ButtonProps { label: string }\nexport function Button(props: ButtonProps) { return helper(props.label); }"
                .into(),
        ),
        (
            "src/use.tsx".into(),
            "import type { ButtonProps } from './button'; export type Use = ButtonProps;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "ButtonProps")
    .expect("finds button props");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/button.tsx"],
        "import { helper } from './helper';\nimport type { ButtonProps } from './types/src';\n\nexport function Button(props: ButtonProps) { return helper(props.label); }\n\nexport type { ButtonProps } from './types/src';\n"
    );
}

#[test]
fn reexports_without_import_when_source_keeps_unrelated_values() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export const label = 'shared'; export type Shared = { id: string };".into(),
        ),
        (
            "src/app.ts".into(),
            "import { type Shared, label } from './shared'; type App = Shared; const current = label;"
                .into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(changes["src/shared.ts"].contains("export const label = 'shared';"));
    assert!(changes["src/shared.ts"].contains("export type { Shared } from './types/src';"));
    assert!(!changes["src/shared.ts"].contains("import type { Shared }"));
}

#[test]
fn keeps_a_blank_line_before_a_trailing_compatibility_export() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/collection/types.ts".into(),
            "export type Kept = { id: string }\nexport type Moved = { name: string }\n".into(),
        ),
        (
            "src/a.ts".into(),
            "import type { Moved } from './collection/types'; type A = Moved;\n".into(),
        ),
        (
            "src/b.ts".into(),
            "import type { Moved } from './collection/types'; type B = Moved;\n".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Moved")
    .expect("finds moved type");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/collection/types.ts"],
        "export type Kept = { id: string }\n\nexport type { Moved } from './types/collection'\n"
    );
}

#[test]
fn merges_candidates_from_multiple_sources_into_one_destination() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/external.ts".into(), "export interface External {}".into()),
        (
            "src/alpha.ts".into(),
            "import type { External } from './external'; export type Alpha = External;".into(),
        ),
        (
            "src/beta.ts".into(),
            "import type { External } from './external'; export type Beta = External;".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Alpha } from './alpha'; import type { Beta } from './beta'; type App = Alpha | Beta;".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| matches!(candidate.symbol_name.as_str(), "Alpha" | "Beta"))
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    let destination = &changes["src/types/src.ts"];
    assert_eq!(destination.matches("import type { External }").count(), 1);
    assert_eq!(
        destination,
        "import type { External } from '../external';\nexport type Alpha = External;\n\nexport type Beta = External;\n"
    );
    assert_eq!(
        changes["src/app.ts"],
        "import type { Alpha, Beta } from './types/src';\n type App = Alpha | Beta;"
    );
}

#[test]
fn wraps_a_merged_import_past_80_when_the_file_has_a_longer_import() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type CatalogRequestParams = string;\nexport type CatalogResponseParams = number;\n"
                .into(),
        ),
        (
            "src/kept.ts".into(),
            "export type AVeryLongKeptNameThatExistsOnlyToExceedTheMergedImportLineLength = boolean;\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { AVeryLongKeptNameThatExistsOnlyToExceedTheMergedImportLineLength } from './kept';\nimport type { CatalogRequestParams } from './feature/schema';\nimport type { CatalogResponseParams } from './feature/schema';\nexport type App = AVeryLongKeptNameThatExistsOnlyToExceedTheMergedImportLineLength | CatalogRequestParams | CatalogResponseParams;\n"
                .into(),
        ),
        (
            "src/other.ts".into(),
            "import type { CatalogRequestParams, CatalogResponseParams } from './feature/schema';\nexport type Other = CatalogRequestParams | CatalogResponseParams;\n"
                .into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| {
        candidate.symbol_name == "CatalogRequestParams"
            || candidate.symbol_name == "CatalogResponseParams"
    })
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import type { AVeryLongKeptNameThatExistsOnlyToExceedTheMergedImportLineLength } from './kept';\nimport type {\n  CatalogRequestParams,\n  CatalogResponseParams,\n} from './feature/types/feature';\nexport type App = AVeryLongKeptNameThatExistsOnlyToExceedTheMergedImportLineLength | CatalogRequestParams | CatalogResponseParams;\n"
    );
}

#[test]
fn wraps_a_long_merged_import() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type VeryLongCatalogRequestParams = string;\nexport type VeryLongCatalogResponseParams = number;\n"
                .into(),
        ),
        (
            "src/kept.ts".into(),
            "export type Kept = boolean;\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import type {\n  Kept,\n} from './kept';\nimport type { VeryLongCatalogRequestParams } from './feature/schema';\nimport type { VeryLongCatalogResponseParams } from './feature/schema';\nexport type App = Kept | VeryLongCatalogRequestParams | VeryLongCatalogResponseParams;\n"
                .into(),
        ),
        (
            "src/other.ts".into(),
            "import type { VeryLongCatalogRequestParams, VeryLongCatalogResponseParams } from './feature/schema';\nexport type Other = VeryLongCatalogRequestParams | VeryLongCatalogResponseParams;\n"
                .into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| {
        candidate.symbol_name == "VeryLongCatalogRequestParams"
            || candidate.symbol_name == "VeryLongCatalogResponseParams"
    })
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import type {\n  Kept,\n} from './kept';\nimport type {\n  VeryLongCatalogRequestParams,\n  VeryLongCatalogResponseParams,\n} from './feature/types/feature';\nexport type App = Kept | VeryLongCatalogRequestParams | VeryLongCatalogResponseParams;\n"
    );
    assert_eq!(
        changes["src/feature/schema.ts"],
        "export type {\n  VeryLongCatalogRequestParams,\n  VeryLongCatalogResponseParams,\n} from './types/feature';\n"
    );
}

#[test]
fn keeps_a_missing_trailing_comma_from_a_multiline_import() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type KeptShape = {\n  id: string\n};\nexport type VeryLongCatalogRequestParams = string;\nexport type VeryLongCatalogResponseParams = number;\n"
                .into(),
        ),
        (
            "src/kept.ts".into(),
            "export type Kept = boolean;\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import type {\n  Kept\n} from './kept';\nimport type { VeryLongCatalogRequestParams } from './feature/schema';\nimport type { VeryLongCatalogResponseParams } from './feature/schema';\nexport type App = Kept | VeryLongCatalogRequestParams | VeryLongCatalogResponseParams;\n"
                .into(),
        ),
        (
            "src/other.ts".into(),
            "import type { VeryLongCatalogRequestParams, VeryLongCatalogResponseParams } from './feature/schema';\nexport type Other = VeryLongCatalogRequestParams | VeryLongCatalogResponseParams;\n"
                .into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| {
        candidate.symbol_name == "VeryLongCatalogRequestParams"
            || candidate.symbol_name == "VeryLongCatalogResponseParams"
    })
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert!(
        changes["src/app.ts"].contains(
            "import type {\n  VeryLongCatalogRequestParams,\n  VeryLongCatalogResponseParams\n} from './feature/types/feature';"
        )
    );
    assert!(changes["src/feature/schema.ts"].contains("VeryLongCatalogResponseParams,\n}"));
}

#[test]
fn merges_type_imports_separated_by_another_statement() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/gate/mechanical.ts".into(),
            "export type CheckRun = { name: string };\n".into(),
        ),
        (
            "src/gate/should-run.ts".into(),
            "export type PullRequestFacts = { number: number };\n".into(),
        ),
        (
            "src/gate/value.ts".into(),
            "export const label = 'gate';\n".into(),
        ),
        (
            "src/gate/gate.ts".into(),
            "import type { CheckRun } from './mechanical';\nimport { label } from './value';\nimport type { PullRequestFacts } from './should-run';\nexport const selected = label;\ntype Gate = CheckRun | PullRequestFacts;\n"
                .into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| {
        matches!(
            candidate.symbol_name.as_str(),
            "CheckRun" | "PullRequestFacts"
        )
    })
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/gate/gate.ts"],
        "import type { CheckRun, PullRequestFacts } from './types/gate';\nimport { label } from './value';\nexport const selected = label;\ntype Gate = CheckRun | PullRequestFacts;\n"
    );
}

#[test]
fn merges_a_moved_import_with_names_split_from_a_later_import() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/devin/env.ts".into(),
            "export type DevinEnv = { token: string };\n".into(),
        ),
        (
            "src/devin/session.ts".into(),
            "export type FetchLike = { fetch: string };\nexport type SessionLayer = { name: string };\n"
                .into(),
        ),
        (
            "src/devin/cleanup.ts".into(),
            "import type { DevinEnv } from './env';\nimport type { FetchLike, SessionLayer } from './session';\ntype App = DevinEnv | FetchLike | SessionLayer;\n"
                .into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| matches!(candidate.symbol_name.as_str(), "DevinEnv" | "FetchLike"))
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/devin/cleanup.ts"],
        "import type { DevinEnv, FetchLike } from './types/devin';\nimport type { SessionLayer } from './session';\ntype App = DevinEnv | FetchLike | SessionLayer;\n"
    );
}

#[test]
fn preserves_the_existing_newline_after_merged_referrer_imports() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/alpha.ts".into(), "export type Alpha = { id: string };\n".into()),
        (
            "src/beta.ts".into(),
            "export interface Beta { enabled: boolean }\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Alpha } from './alpha';\nimport type { Beta } from './beta';\nexport type App = Alpha | Beta;\n"
                .into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| matches!(candidate.symbol_name.as_str(), "Alpha" | "Beta"))
    .collect::<Vec<_>>();

    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");

    assert_eq!(
        changes["src/app.ts"],
        "import type { Alpha, Beta } from './types/src';\nexport type App = Alpha | Beta;\n"
    );
}

#[test]
fn moves_multiple_candidates_from_one_source_file() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export type First = {}; export interface Second { id: string }".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { First, Second } from './shared'; type App = First | Second;".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans");
    assert_eq!(candidates.len(), 2);
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert!(changes["src/shared.ts"].contains("export type { First, Second } from './types/src';"));
    assert_eq!(
        changes["src/types/src.ts"],
        "export type First = {};\nexport interface Second { id: string }\n"
    );
    assert_eq!(
        changes["src/app.ts"],
        "import type { First, Second } from './types/src'; type App = First | Second;"
    );
}

#[test]
fn preserves_blank_lines_between_moved_declarations() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export interface Alpha { id: string }\nexport type Beta = { enabled: boolean };\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Alpha, Beta } from './shared'; type App = Alpha | Beta;".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans");
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/types/src.ts"],
        "export interface Alpha { id: string }\n\nexport type Beta = { enabled: boolean };\n"
    );
}

#[test]
fn preserves_multiple_referrer_updates_in_one_file() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/alpha/first.ts".into(),
            "export type First = {};".into(),
        ),
        (
            "src/beta/second.ts".into(),
            "export type Second = {};".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { First } from './alpha/first'; import type { Second } from './beta/second'; type App = First | Second;"
                .into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans");
    assert_eq!(candidates.len(), 2);
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import type { First } from './alpha/types/alpha'; import type { Second } from './beta/types/beta'; type App = First | Second;"
    );
}

#[test]
fn rewrites_dependencies_between_moved_candidates() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/alpha/first.ts".into(),
            "import type { Second } from '../beta/second'; export type First = Second;".into(),
        ),
        (
            "src/beta/second.ts".into(),
            "export type Second = {};".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { First } from './alpha/first'; type App = First;".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans");
    assert_eq!(candidates.len(), 2);
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert!(changes["src/alpha/first.ts"].contains("export type { First } from './types/alpha';"));
    assert!(changes["src/beta/second.ts"].contains("export type { Second } from './types/beta';"));
    assert_eq!(
        changes["src/alpha/types/alpha.ts"],
        "import type { Second } from '../../beta/types/beta';\nexport type First = Second;\n"
    );
    assert_eq!(
        changes["src/app.ts"],
        "import type { First } from './alpha/types/alpha'; type App = First;"
    );
}

#[test]
fn moves_same_file_type_dependencies_in_source_order() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "type Detail = string; export type Shared = Detail;".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(changes["src/shared.ts"].contains("export type { Shared } from './types/src';"));
    assert_eq!(
        changes["src/types/src.ts"],
        "type Detail = string;\nexport type Shared = Detail;\n"
    );
}

#[test]
fn moves_leading_documentation_comments_with_declarations() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "/** Local helper. */\ntype Detail = string;\n/** Shared public type. */\nexport type Shared = Detail;"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/types/src.ts"],
        "/** Local helper. */\ntype Detail = string;\n\n/** Shared public type. */\nexport type Shared = Detail;\n"
    );
    assert_eq!(
        changes["src/shared.ts"],
        "export type { Shared } from './types/src';\n"
    );
}

#[test]
fn keeps_comments_attached_to_imports_when_inserting_and_splitting() {
    let header = "// comment for the original import\nimport { helper } from './z';\n\nexport const value = helper;\n";
    let inserted =
        super::format::insert_leading_import(header, "import type { Shared } from './a';");
    assert_eq!(
        inserted,
        "// comment for the original import\nimport { helper } from './z';\nimport type { Shared } from './a';\n\nexport const value = helper;\n"
    );
    let source = "import { helper } from './helper'; // helper explanation\n\nexport const use = (value: Shared) => helper(value);\n";
    let result = super::compatibility::place_remaining_source_compatibility(
        source,
        "import type { Shared } from './types/shared';",
        "export type { Shared } from './types/shared';",
    );
    assert_eq!(
        result,
        "import { helper } from './helper'; // helper explanation\nimport type { Shared } from './types/shared';\n\nexport const use = (value: Shared) => helper(value);\n\nexport type { Shared } from './types/shared';\n"
    );

    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/shared.ts".into(), "export type Shared = { id: string };\nexport const value = 1;\n".into()),
        ("src/app.ts".into(), "import { type Shared, value } from './shared'; // keep with value\n\nexport const run = (item: Shared) => value;\n".into()),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "Shared")
    .unwrap();
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/app.ts"],
        "import { value } from './shared'; // keep with value\nimport type { Shared } from './types/src';\n\nexport const run = (item: Shared) => value;\n"
    );
}

#[test]
fn preserves_unmodified_import_groups_without_a_compatibility_import() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/shared.ts".into(), "import { external } from 'external';\n\nimport { internal } from './internal';\n\nexport const label = external + internal;\nexport type Shared = { id: string };\n".into()),
        ("src/app.ts".into(), "import type { Shared } from './shared';\nexport type App = Shared;\n".into()),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "Shared")
    .unwrap();
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/shared.ts"],
        "import { external } from 'external';\n\nimport { internal } from './internal';\n\nexport const label = external + internal;\n\nexport type { Shared } from './types/src';\n"
    );
}

#[test]
fn keeps_comments_when_multiple_type_imports_share_a_destination() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/shared.ts".into(), "export type A = string;\nexport type B = number;\n".into()),
        ("src/app.ts".into(), "import type { A /* important */ } from './shared';\nimport type { B } from './shared'; // keep for B\n\nexport type Both = [A, B];\n".into()),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .filter(|c| c.symbol_name == "A" || c.symbol_name == "B")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).unwrap();
    assert_eq!(
        changes["src/app.ts"],
        "import type { A /* important */ } from './types/src';\nimport type { B } from './types/src'; // keep for B\n\nexport type Both = [A, B];\n"
    );
}

#[test]
fn keeps_comments_inside_a_mixed_import_via_compatibility_export() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/shared.ts".into(), "export type Shared = { id: string };\nexport const value = 1;\n".into()),
        ("src/app.ts".into(), "import {\n  type Shared, // keep explanation\n  value,\n} from './shared';\n\nexport const run = (item: Shared) => value;\n".into()),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "Shared")
    .unwrap();
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes.get("src/app.ts").unwrap_or(&sources["src/app.ts"]),
        &sources["src/app.ts"]
    );
    assert!(changes["src/shared.ts"].contains("export type { Shared }"));
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"noEmit":true},"include":["src/**/*.ts"]}"#,
    )
    .unwrap();
    for (path, source) in &sources {
        fs::write(root.join(path), source).unwrap();
    }
    write_file_transaction(root, &changes).unwrap();
    let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/typescript/bin/tsc");
    if compiler.exists() {
        let output = std::process::Command::new("node")
            .arg(compiler)
            .arg("--project")
            .arg(root.join("tsconfig.json"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn preserves_comments_inside_a_mixed_barrel_reexport() {
    for barrel in [
        "export { type A /* public contract */, value } from './schema';\n",
        "// public contract\nexport { type A, value } from './schema';\n",
        "export { type A, value } from './schema'; // public contract\n",
    ] {
        let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/feature/schema.ts".into(), "export type A = { id: string };\nexport const value = 1;\n".into()),
        ("src/feature/index.ts".into(), barrel.into()),
        ("src/app.ts".into(), "import { type A, value } from './feature';\nexport const result = (item: A) => value;\n".into()),
    ]);
        let candidate = scan_project_files(
            sources
                .iter()
                .map(|(p, s)| (p.clone(), s.clone()))
                .collect(),
        )
        .unwrap()
        .into_iter()
        .find(|c| c.symbol_name == "A")
        .unwrap();
        let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
        assert_eq!(
            changes
                .get("src/feature/index.ts")
                .unwrap_or(&sources["src/feature/index.ts"]),
            &sources["src/feature/index.ts"]
        );
        assert!(changes["src/feature/schema.ts"].contains("export type { A }"));
    }
}

#[test]
fn preserves_crlf_when_splitting_a_barrel_reexport() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/feature/schema.ts".into(), "export type A = { id: string };\r\nexport const value = 1;\r\n".into()),
        ("src/feature/index.ts".into(), "export { type A, value } from './schema';\r\n".into()),
        ("src/app.ts".into(), "import { type A, value } from './feature';\r\nexport const result = (item: A) => value;\r\n".into()),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "A")
    .unwrap();
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/feature/index.ts"],
        "export { value } from './schema';\r\nexport type { A } from './types/feature';\r\n"
    );
    assert!(
        !changes["src/feature/index.ts"]
            .replace("\r\n", "")
            .contains('\n')
    );
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("src/feature")).unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"noEmit":true},"include":["src/**/*.ts"]}"#,
    )
    .unwrap();
    for (path, source) in &sources {
        fs::write(root.join(path), source).unwrap();
    }
    write_file_transaction(root, &changes).unwrap();
    let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/typescript/bin/tsc");
    if compiler.exists() {
        let output = std::process::Command::new("node")
            .arg(compiler)
            .arg("--project")
            .arg(root.join("tsconfig.json"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn preserves_crlf_in_rewritten_sources_and_generated_files() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/shared.ts".into(), "export type Shared = { id: string };\r\nexport const value = (item: Shared) => item.id;\r\n".into()),
        ("src/app.ts".into(), "import { type Shared, value } from \"./shared\";\r\n\r\nexport const run = (item: Shared) => value(item);\r\n".into()),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "Shared")
    .unwrap();
    let changes = plan_extraction_rewrite(&[candidate], &sources).unwrap();
    assert_eq!(
        changes["src/app.ts"],
        "import { value } from \"./shared\";\r\nimport type { Shared } from \"./types/src\";\r\n\r\nexport const run = (item: Shared) => value(item);\r\n"
    );
    assert_eq!(
        changes["src/types/src.ts"],
        "export type Shared = { id: string };\r\n"
    );
    for (path, content) in &changes {
        assert!(
            !content.replace("\r\n", "").contains('\n'),
            "{path}: {content:?}"
        );
    }
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"noEmit":true},"include":["src/**/*.ts"]}"#,
    )
    .unwrap();
    for (path, source) in &sources {
        fs::write(root.join(path), source).unwrap();
    }
    write_file_transaction(root, &changes).unwrap();
    let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/typescript/bin/tsc");
    if compiler.exists() {
        let output = std::process::Command::new("node")
            .arg(compiler)
            .arg("--project")
            .arg(root.join("tsconfig.json"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn checks_generated_import_groups_with_oxfmt_when_available() {
    let formatter = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
        "../../node_modules/.bin/oxfmt.cmd"
    } else {
        "../../node_modules/.bin/oxfmt"
    });
    if !formatter.exists() {
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("app/hooks/cart")).unwrap();
    fs::create_dir_all(root.join("app/components")).unwrap();
    fs::write(root.join(".oxfmtrc.json"), r#"{"semi":false,"singleQuote":true,"sortImports":{"groups":["builtin","external",["internal","subpath"],["parent","sibling","index"],"unknown"]}}"#).unwrap();
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("app/hooks/cart/use-cart.ts".into(), "import { hook } from 'react'\n\nimport { helper } from '~/helpers'\n\nexport type Draft = { id: string }\nexport const useDraft = (draft: Draft) => helper(hook, draft)\n".into()),
        ("app/components/view.ts".into(), "import { type Draft, useDraft } from '~/hooks/cart/use-cart'\nimport { other } from '~/zebra'\n\nexport const view = (draft: Draft) => useDraft(draft) + other\n".into()),
    ]);
    for (path, source) in &sources {
        fs::write(root.join(path), source).unwrap();
    }
    let check = || {
        std::process::Command::new(&formatter)
            .current_dir(root)
            .args([
                "--check",
                "app/hooks/cart/use-cart.ts",
                "app/components/view.ts",
            ])
            .output()
            .unwrap()
    };
    let baseline = check();
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stdout)
    );
    let aliases = BTreeMap::from([("~/*".into(), vec!["app/*".into()])]);
    let candidate = scan_project_files_with_aliases(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
        None,
        &aliases,
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "Draft")
    .unwrap();
    let changes = plan_extraction_rewrite_with_aliases(
        &[candidate],
        &sources,
        None,
        &aliases,
        CompatibilityReexportQuoteStyle::Apply,
    )
    .unwrap();
    write_file_transaction(root, &changes).unwrap();
    let after = check();
    assert!(
        after.status.success(),
        "{}",
        String::from_utf8_lossy(&after.stdout)
    );
    let generated = std::process::Command::new(&formatter)
        .current_dir(root)
        .args(["--check", "app/hooks/cart/types/cart.ts"])
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stdout)
    );
}

#[test]
fn inserts_new_imports_without_disrupting_existing_import_groups() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("app/hooks/cart/use-cart.ts".into(), "import { hook } from 'react'\n\nimport { helper } from '~/helpers'\n\nexport type Draft = { id: string }\nexport const useDraft = (draft: Draft) => helper(hook, draft)\n".into()),
        ("app/components/view.ts".into(), "import { type Draft, useDraft } from '~/hooks/cart/use-cart'\nimport { other } from '~/zebra'\n\nexport const view = (draft: Draft) => useDraft(draft) + other\n".into()),
        ("app/components/alt.ts".into(), "import { type Draft, useDraft } from \"@/hooks/cart/use-cart\";\n\nexport const alt = (draft: Draft) => useDraft(draft);\n".into()),
    ]);
    let aliases = BTreeMap::from([
        ("~/*".into(), vec!["app/*".into()]),
        ("@/*".into(), vec!["app/*".into()]),
    ]);
    let candidate = scan_project_files_with_aliases(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
        None,
        &aliases,
    )
    .unwrap()
    .into_iter()
    .find(|c| c.symbol_name == "Draft")
    .unwrap();
    let changes = plan_extraction_rewrite_with_aliases(
        &[candidate],
        &sources,
        None,
        &aliases,
        CompatibilityReexportQuoteStyle::Apply,
    )
    .unwrap();
    assert_eq!(
        changes["app/hooks/cart/use-cart.ts"],
        "import { hook } from 'react'\n\nimport { helper } from '~/helpers'\n\nimport type { Draft } from './types/cart'\n\nexport const useDraft = (draft: Draft) => helper(hook, draft)\n\nexport type { Draft } from './types/cart'\n"
    );
    assert_eq!(
        changes["app/components/view.ts"],
        "import type { Draft } from '~/hooks/cart/types/cart'\nimport { useDraft } from '~/hooks/cart/use-cart'\nimport { other } from '~/zebra'\n\nexport const view = (draft: Draft) => useDraft(draft) + other\n"
    );
    assert_eq!(
        changes["app/components/alt.ts"],
        "import type { Draft } from \"@/hooks/cart/types/cart\";\nimport { useDraft } from \"@/hooks/cart/use-cart\";\n\nexport const alt = (draft: Draft) => useDraft(draft);\n"
    );
}

#[test]
fn places_a_compatibility_import_after_the_file_banner() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/banner/banner.ts".into(),
            "/** File banner. */\n\nexport const label = (value: Shared) => value;\nexport type Shared = { id: string };\n"
                .into(),
        ),
        (
            "src/directive/directive.ts".into(),
            "/* eslint-disable camelcase */\n\nexport const label = (value: Shared) => value;\nexport type Shared = { id: string };\n"
                .into(),
        ),
        (
            "src/attached/attached.ts".into(),
            "/** Uses the shared value. */\nexport const label = (value: Shared) => value;\nexport type Shared = { id: string };\n"
                .into(),
        ),
        (
            "src/above/other.ts".into(),
            "export const other = 1;\n".into(),
        ),
        (
            "src/above/above.ts".into(),
            "/** File banner. */\nimport { other } from './other';\nexport const label = (value: Shared) => value;\nexport type Shared = { id: string };\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared as Banner } from './banner/banner';\nimport type { Shared as Directive } from './directive/directive';\nimport type { Shared as Attached } from './attached/attached';\nimport type { Shared as Above } from './above/above';\ntype App = Banner | Directive | Attached | Above;\n"
                .into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| candidate.symbol_name == "Shared")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/banner/banner.ts"],
        "/** File banner. */\n\nimport type { Shared } from './types/banner';\n\nexport const label = (value: Shared) => value;\n\nexport type { Shared } from './types/banner';\n"
    );
    assert_eq!(
        changes["src/directive/directive.ts"],
        "/* eslint-disable camelcase */\n\nimport type { Shared } from './types/directive';\n\nexport const label = (value: Shared) => value;\n\nexport type { Shared } from './types/directive';\n"
    );
    assert_eq!(
        changes["src/attached/attached.ts"],
        "import type { Shared } from './types/attached';\n\n/** Uses the shared value. */\nexport const label = (value: Shared) => value;\n\nexport type { Shared } from './types/attached';\n"
    );
    assert_eq!(
        changes["src/above/above.ts"],
        "/** File banner. */\nimport { other } from './other';\nimport type { Shared } from './types/above';\n\nexport const label = (value: Shared) => value;\n\nexport type { Shared } from './types/above';\n"
    );
}

#[test]
fn keeps_specifier_order_and_the_blank_line_under_imports() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/order/source.ts".into(),
            "import type { Zoo } from './apple';\nimport type { Apple } from './zoo';\n\nexport const label = 'order';\nexport type Shared = { id: string };\n"
                .into(),
        ),
        (
            "src/keep/source.ts".into(),
            "import type { Other } from './zzz';\nexport const label = (value: Shared) => value;\nexport type Shared = { id: string };\n"
                .into(),
        ),
        ("src/keep/zzz.ts".into(), "export type Other = string;\n".into()),
        (
            "src/app.ts".into(),
            "import type { Shared as Ordered } from './order/source';\nimport type { Shared } from './keep/source';\ntype App = Ordered | Shared;\n"
                .into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| candidate.symbol_name == "Shared")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/order/source.ts"],
        "import type { Zoo } from './apple';\nimport type { Apple } from './zoo';\n\nexport const label = 'order';\n\nexport type { Shared } from './types/order';\n"
    );
    assert_eq!(
        changes["src/keep/source.ts"],
        "import type { Shared } from './types/keep';\nimport type { Other } from './zzz';\n\nexport const label = (value: Shared) => value;\n\nexport type { Shared } from './types/keep';\n"
    );
}

#[test]
fn keeps_a_file_banner_when_other_code_stays() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "/** File banner. */\n\n/** Shared public type. */\nexport type Shared = { id: string };\nexport const label = 'shared';\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared';\nexport const value: Shared = { id: '1' };\n"
                .into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/types/src.ts"],
        "/** Shared public type. */\nexport type Shared = { id: string };\n"
    );
    assert!(changes["src/shared.ts"].contains("/** File banner. */"));
    assert!(!changes["src/types/src.ts"].contains("File banner"));
    assert!(changes["src/shared.ts"].contains("export const label = 'shared';"));
}

#[test]
fn moves_default_type_import_dependencies() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/external.ts".into(),
            "export default type External = string;".into(),
        ),
        (
            "src/shared.ts".into(),
            "import type External from './external'; export type Shared = External;".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/types/src.ts"],
        "import type External from '../external';\nexport type Shared = External;\n"
    );
}

#[test]
fn reexports_exported_local_dependencies_from_the_source_module() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export type Local = { id: string };\nexport type Shared = { local: Local };".into(),
        ),
        (
            "src/consumer.ts".into(),
            "import type { Shared } from './shared'; export type Consumer = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(
        changes.contains_key("src/shared.ts"),
        "missing shared.ts in {:?}",
        changes.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        changes["src/shared.ts"],
        "export type { Local, Shared } from './types/src';\n"
    );
}

#[test]
fn does_not_introduce_runtime_import_when_removing_namespace_type_imports() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/external.ts".into(),
            "export interface Base { id: string }\nexport type Item = { label: string };".into(),
        ),
        (
            "src/shared.ts".into(),
            "import type * as ExternalTypes from './external';\nexport interface Shared extends ExternalTypes.Base { item: ExternalTypes.Item }"
                .into(),
        ),
        (
            "src/consumer.ts".into(),
            "import type { Shared } from './shared'; export type Consumer = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/shared.ts"],
        "export type { Shared } from './types/src';\n"
    );
    assert_eq!(
        changes["src/types/src.ts"],
        "import type * as ExternalTypes from '../external';\nexport interface Shared extends ExternalTypes.Base { item: ExternalTypes.Item }\n"
    );
}

#[test]
fn moves_namespace_type_import_dependencies() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/external.ts".into(),
            "export interface Value { id: string }".into(),
        ),
        (
            "src/shared.ts".into(),
            "import type * as External from './external'; export type Shared = External.Value;"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/types/src.ts"],
        "import type * as External from '../external';\nexport type Shared = External.Value;\n"
    );
}

#[test]
fn preserves_named_and_namespace_type_imports_from_one_module() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/external.ts".into(),
            "export interface Value { id: string } export type Detail = string;".into(),
        ),
        (
            "src/shared.ts".into(),
            "import type * as External from './external'; import type { Detail } from './external'; export type Shared = External.Value | Detail;"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/types/src.ts"],
        "import type * as External from '../external';\nimport type { Detail } from '../external';\nexport type Shared = External.Value | Detail;\n"
    );
}

#[test]
fn plans_alias_import_updates_from_tsconfig_patterns() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type Schema = {};".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Schema } from '@/feature/schema'; type App = Schema;".into(),
        ),
    ]);
    let candidate = crate::scan::scan_project_files_with_aliases(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
        Some("."),
        &BTreeMap::from([("@/*".into(), vec!["src/*".into()])]),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite_with_aliases(
        &[candidate],
        &sources,
        Some("."),
        &BTreeMap::from([("@/*".into(), vec!["src/*".into()])]),
        CompatibilityReexportQuoteStyle::Apply,
    )
    .expect("plans");
    assert!(changes["src/app.ts"].contains("'@/feature/types/feature'"));
}

#[test]
fn preserves_dot_prefixed_aliases_when_rewriting_imports() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type Schema = {};".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Schema } from '.types/feature/schema'; type App = Schema;".into(),
        ),
    ]);
    let aliases = BTreeMap::from([(".types/*".into(), vec!["src/*".into()])]);
    let candidate = scan_project_files_with_aliases(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
        Some("."),
        &aliases,
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Schema")
    .expect("finds schema");
    let changes = plan_extraction_rewrite_with_aliases(
        &[candidate],
        &sources,
        Some("."),
        &aliases,
        CompatibilityReexportQuoteStyle::Apply,
    )
    .expect("plans");
    assert!(changes["src/app.ts"].contains("'.types/feature/types/feature'"));
}

#[test]
fn keeps_tilde_and_at_aliases_when_targets_are_dot_slash() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "services/domains/catalog-v2/types.ts".into(),
            "export type Item = { id: string };\n".into(),
        ),
        (
            "features/catalog/view.ts".into(),
            "import type { Item } from '~/services/domains/catalog-v2/types';\nexport type View = Item;\n"
                .into(),
        ),
        (
            "features/catalog/list.ts".into(),
            "import type { Item } from '@/services/domains/catalog-v2/types.js';\nexport type List = Item;\n"
                .into(),
        ),
    ]);
    let paths = BTreeMap::from([
        ("@/*".into(), vec!["./*".into()]),
        ("~/*".into(), vec!["./*".into()]),
    ]);
    let candidates = crate::scan::scan_project_files_with_aliases(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
        Some("."),
        &paths,
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| candidate.symbol_name == "Item")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite_with_aliases(
        &candidates,
        &sources,
        Some("."),
        &paths,
        CompatibilityReexportQuoteStyle::Apply,
    )
    .expect("plans");
    assert!(
        changes["features/catalog/view.ts"]
            .contains("from '~/services/domains/catalog-v2/types/catalog-v2'")
    );
    assert!(
        changes["features/catalog/list.ts"]
            .contains("from '@/services/domains/catalog-v2/types/catalog-v2.js'")
    );
}

#[test]
fn preserves_unmoved_type_in_a_shared_barrel_reexport() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type A = { a: string };\nexport type B = { b: number };\n".into(),
        ),
        (
            "src/feature/index.ts".into(),
            "export type { A, B } from './schema';\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { A, B } from './feature';\nexport type Both = [A, B];\n".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "A")
    .expect("finds A");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    let barrel = changes
        .get("src/feature/index.ts")
        .unwrap_or(&sources["src/feature/index.ts"]);
    assert!(barrel.contains("B } from './schema'"), "{barrel}");
}

#[test]
fn keeps_value_exports_with_the_same_name_as_a_moved_type() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type A = { a: string };\nexport const A = { a: 'value' };\n".into(),
        ),
        (
            "src/feature/index.ts".into(),
            "export { A } from './schema';\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import { A } from './feature';\nexport const value: A = A;\n".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "A")
    .expect("finds A");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    let barrel = changes
        .get("src/feature/index.ts")
        .unwrap_or(&sources["src/feature/index.ts"]);
    assert_eq!(barrel, &sources["src/feature/index.ts"]);
}

#[test]
fn preserves_barrel_exports_across_selected_type_moves() {
    for (barrel, imported_a, move_b) in [
        ("export type { A, B } from './schema';\n", "A", false),
        ("export { type A, type B } from './schema';\n", "A", false),
        ("export { type A, type B } from './schema';\n", "A", true),
        (
            "export { type A as PublicA, type B } from './schema';\n",
            "PublicA",
            false,
        ),
        ("export type { A, B } from './schema';\n", "A", true),
        (
            "export type { A as PublicA, B } from './schema';\n",
            "PublicA",
            false,
        ),
        (
            "export type { A as PublicA, B } from './schema';\n",
            "PublicA",
            true,
        ),
        ("export * from './schema';\n", "A", false),
    ] {
        let fixture = tempfile::tempdir().expect("creates fixture");
        let root = fixture.path();
        fs::create_dir_all(root.join("src/feature")).expect("creates directory");
        fs::write(
            root.join("tsconfig.json"),
            r#"{"compilerOptions":{"strict":true,"noEmit":true},"include":["src/**/*.ts"]}"#,
        )
        .expect("writes config");
        fs::write(
            root.join("src/feature/schema.ts"),
            "export type A = { a: string };\nexport type B = { b: number };\nexport const label = 'kept';\n",
        )
        .expect("writes declarations");
        fs::write(root.join("src/feature/index.ts"), barrel).expect("writes barrel");
        fs::write(
            root.join("src/app.ts"),
            format!("import type {{ {imported_a}, B }} from './feature';\nexport const a: {imported_a} = {{ a: 'ok' }};\nexport const b: B = {{ b: 1 }};\n"),
        )
        .expect("writes consumer");
        if barrel.starts_with("export *") {
            fs::write(
                root.join("src/value.ts"),
                "import { label } from './feature';\nexport const value = label;\n",
            )
            .expect("writes value consumer");
        }
        let candidates = apply_placement(
            scan_tsconfig(&root.join("tsconfig.json")).expect("scans"),
            PlacementStyle::Collocation,
        )
        .into_iter()
        .filter(|c| c.symbol_name == "A" || (move_b && c.symbol_name == "B"))
        .collect::<Vec<_>>();
        assert_eq!(candidates.len(), if move_b { 2 } else { 1 }, "{barrel}");
        let sources = discover_typescript_resolution_files(root, true)
            .expect("discovers")
            .into_iter()
            .map(|path| {
                let name = path
                    .strip_prefix(root)
                    .expect("relative")
                    .to_string_lossy()
                    .replace('\\', "/");
                (name, fs::read_to_string(path).expect("reads"))
            })
            .collect::<BTreeMap<_, _>>();
        let changes = plan_extraction_rewrite(&candidates, &sources)
            .expect("plans without conflicting edits");
        write_file_transaction(root, &changes).expect("writes changes");
        let source = fs::read_to_string(root.join("src/feature/schema.ts")).expect("reads source");
        assert_eq!(source.contains("export type B ="), !move_b, "{barrel}");
        let rewritten_barrel =
            fs::read_to_string(root.join("src/feature/index.ts")).expect("reads barrel");
        if barrel.starts_with("export *") {
            assert_eq!(rewritten_barrel, barrel);
        } else if !move_b && !barrel.starts_with("export {") {
            assert_eq!(rewritten_barrel, barrel);
        }
        let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../node_modules/typescript/bin/tsc");
        if compiler.exists() {
            let output = std::process::Command::new("node")
                .arg(compiler)
                .arg("--project")
                .arg(root.join("tsconfig.json"))
                .output()
                .expect("runs TypeScript");
            assert!(
                output.status.success(),
                "{barrel}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}

#[test]
fn plans_barrel_export_updates_without_rewriting_referrers() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type Schema = {};".into(),
        ),
        (
            "src/feature/index.ts".into(),
            "export type { Schema } from './schema';".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Schema } from './feature'; type App = Schema;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(changes["src/feature/index.ts"].contains("'./types/feature'"));
    assert!(!changes.contains_key("src/app.ts"));
}

#[test]
fn keeps_a_tilde_alias_when_a_barrel_reexports_a_moved_type() {
    let aliases = BTreeMap::from([("~/*".into(), vec!["./*".into()])]);
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "services/domains/csv-order/types.ts".into(),
            "export type CsvOrder = { id: number };\n".into(),
        ),
        (
            "features/csv-order/types/group.ts".into(),
            "export { STATUS } from '~/services/domains/csv-order/types';\nexport type { CsvOrder } from '~/services/domains/csv-order/types';\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { CsvOrder } from '../features/csv-order/types/group'; type App = CsvOrder;\n"
                .into(),
        ),
    ]);
    let candidate = scan_project_files_with_aliases(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
        Some("."),
        &aliases,
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "CsvOrder")
    .expect("finds csv order");
    let changes = plan_extraction_rewrite_with_aliases(
        &[candidate],
        &sources,
        Some("."),
        &aliases,
        CompatibilityReexportQuoteStyle::Apply,
    )
    .expect("plans");
    let group = &changes["features/csv-order/types/group.ts"];
    assert!(group.contains("export { STATUS } from '~/services/domains/csv-order/types'"));
    assert!(
        group.contains(
            "export type { CsvOrder } from '~/services/domains/csv-order/types/csv-order'"
        )
    );
}

#[test]
fn plans_wildcard_barrel_export_updates() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type Schema = {};".into(),
        ),
        (
            "src/feature/index.ts".into(),
            "export * from './schema';".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Schema } from './feature'; type App = Schema;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/feature/index.ts"],
        "export * from './types/feature';"
    );
    assert!(!changes.contains_key("src/app.ts"));
}

#[test]
fn preserves_namespace_type_importers_through_source_reexports() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/schema.ts".into(), "export type Schema = {};".into()),
        (
            "src/app.ts".into(),
            "import type * as Types from './schema'; type App = Types.Schema;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(!changes.contains_key("src/app.ts"));
    assert_eq!(
        changes["src/schema.ts"],
        "export type { Schema } from './types/src';\n"
    );
}

#[test]
fn splits_mixed_inline_type_importers() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export const label = 'shared'; export type Shared = { id: string };".into(),
        ),
        (
            "src/app.ts".into(),
            "import { type Shared, label } from './shared'; type App = Shared; const current = label;"
                .into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import { label } from './shared';\nimport type { Shared } from './types/src';\n\n type App = Shared; const current = label;"
    );
}

#[test]
fn splits_default_and_inline_type_importers() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "const label = 'shared'; export default label; export type Shared = { id: string };"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import label, { type Shared } from './shared'; type App = Shared; const current = label;"
                .into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import label from './shared';\nimport type { Shared } from './types/src';\n\n type App = Shared; const current = label;"
    );
}

#[test]
fn splits_partially_moved_inline_type_importers() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export type Other = string; export type Shared = { id: string };".into(),
        ),
        (
            "src/app.ts".into(),
            "import { type Other, type Shared } from './shared'; type App = Shared | Other;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import { type Other } from './shared';\nimport type { Shared } from './types/src';\n\n type App = Shared | Other;"
    );
}

#[test]
fn splits_one_import_when_several_bindings_move_together() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export type Left = string;\nexport type Right = number;\nexport type Kept = boolean;\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Left, Right, Kept } from './shared';\ntype App = Left | Right | Kept;\n"
                .into(),
        ),
        (
            "src/other.ts".into(),
            "import type { Left, Right } from './shared';\ntype Other = Left | Right;\n".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| candidate.symbol_name == "Left" || candidate.symbol_name == "Right")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import type { Kept } from './shared';\nimport type { Left, Right } from './types/src';\n\ntype App = Left | Right | Kept;\n"
    );
    assert_eq!(
        changes["src/other.ts"],
        "import type { Left, Right } from './types/src';\ntype Other = Left | Right;\n"
    );
}

#[test]
fn keeps_a_single_type_marker_when_a_moved_dependency_leaves_an_import() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/deps.ts".into(),
            "export type Needed = string;\nexport type Kept = number;\n".into(),
        ),
        (
            "src/shared.ts".into(),
            "import type {\n  Needed,\n  Kept,\n} from './deps';\nexport type Shared = Needed;\nexport const label: Kept = 1;\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared';\nexport const value: Shared = 'value';\n"
                .into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    let source = &changes["src/shared.ts"];
    assert!(source.contains("import type { Kept } from './deps';"));
    assert!(!source.contains("import type { type "));
    assert!(source.contains("export const label: Kept = 1;"));
}

#[test]
fn moves_a_private_dependency_when_the_source_keeps_other_code() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "type Money = { cents: number };\nexport type OrderSummary = { total: Money };\nexport const label = 'order';\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { OrderSummary } from './shared';\nexport const selected: OrderSummary = { total: { cents: 1 } };\n"
                .into(),
        ),
    ]);
    let candidate = shared_candidate(&sources, "OrderSummary");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    let destination = &changes["src/types/src.ts"];
    assert!(destination.contains("type Money = { cents: number };"));
    assert!(destination.contains("export type OrderSummary = { total: Money };"));
    assert!(!changes["src/shared.ts"].contains("type Money"));
    assert!(changes["src/shared.ts"].contains("export const label = 'order';"));
    assert!(changes["src/shared.ts"].contains("export type { OrderSummary }"));
}

#[test]
fn copies_a_private_dependency_that_remaining_declarations_still_use() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "type IsUnion<Value> = Value;\nexport type SinglePropertyKey<Key> = IsUnion<Key>;\nexport type KeyPresences<Key> = IsUnion<Key> | SinglePropertyKey<Key>;\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { SinglePropertyKey } from './shared';\nexport type Selected<Key> = SinglePropertyKey<Key>;\n"
                .into(),
        ),
    ]);
    let candidate = shared_candidate(&sources, "SinglePropertyKey");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    let destination = &changes["src/types/src.ts"];
    let source = &changes["src/shared.ts"];
    assert!(destination.contains("type IsUnion<Value> = Value;"));
    assert!(destination.contains("export type SinglePropertyKey<Key> = IsUnion<Key>;"));
    assert!(source.contains("type IsUnion<Value> = Value;"));
    assert!(
        source.contains("export type KeyPresences<Key> = IsUnion<Key> | SinglePropertyKey<Key>;")
    );
    assert!(source.contains("import type { SinglePropertyKey }"));
    assert!(!source.contains("import type { IsUnion"));
    assert!(source.contains("export type { SinglePropertyKey }"));
}

fn shared_candidate(
    sources: &BTreeMap<String, String>,
    symbol_name: &str,
) -> crate::model::Candidate {
    scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == symbol_name)
    .expect("finds shared")
}

#[test]
fn inserts_compatibility_imports_around_semicolon_free_statements() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/types.ts".into(), "export type Cycle = 'day'\n".into()),
        (
            "src/common.ts".into(),
            "import { DELIVERY } from '../constants'\nimport type { Cycle } from './types'\n\nexport const format = (dates: { monthInterval: number; dayOfMonth: number }[]): string => {\n  return String(dates.length)\n}\n\nexport type FrequencyFormatParams = {\n  cycleType: Cycle\n}\n\nexport const label = (params: FrequencyFormatParams): Cycle => {\n  return params.cycleType\n}\n"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type { FrequencyFormatParams } from './common'\nexport const value: FrequencyFormatParams = { cycleType: 'day' }\n"
                .into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "FrequencyFormatParams")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    let source = &changes["src/common.ts"];
    assert!(source.contains("import { DELIVERY } from '../constants'"));
    assert!(source.contains("dates: { monthInterval: number; dayOfMonth: number }[]"));
    assert!(!source.contains("number;\nimport"));
    assert!(source.contains("import type { FrequencyFormatParams }"));
    assert!(source.contains("export type { FrequencyFormatParams }"));
    assert!(
        source
            .lines()
            .filter(|line| {
                line.contains("import type { FrequencyFormatParams }")
                    || line.contains("export type { FrequencyFormatParams }")
            })
            .all(|line| !line.trim_end().ends_with(';'))
    );
    let destination = &changes["src/types/src.ts"];
    assert!(
        destination
            .lines()
            .filter(|line| line.contains("import type"))
            .all(|line| !line.trim_end().ends_with(';'))
    );
}

#[test]
fn splits_a_semicolon_free_import_without_adding_semicolons() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export type Left = string\nexport type Kept = boolean\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Left, Kept } from './shared'\ntype App = Left | Kept\n".into(),
        ),
        (
            "src/other.ts".into(),
            "import type { Left } from './shared'\ntype Other = Left\n".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| candidate.symbol_name == "Left")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import type { Kept } from './shared'\nimport type { Left } from './types/src'\n\ntype App = Left | Kept\n"
    );
    assert!(!changes["src/shared.ts"].contains(';'));
    assert!(!changes["src/types/src.ts"].contains(';'));
}

#[test]
fn moves_a_rewritten_type_import_to_its_specifier_position() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/common.ts".into(),
            "export type Frequency = string;\n".into(),
        ),
        (
            "src/detail.ts".into(),
            "export const format = 1;\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import { type Frequency } from './common';\nimport { format } from './detail';\ntype App = Frequency;\n"
                .into(),
        ),
        (
            "src/again.ts".into(),
            "import type { Frequency } from './common';\ntype Again = Frequency;\n".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| candidate.symbol_name == "Frequency")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import { format } from './detail';\nimport type { Frequency } from './types/src';\ntype App = Frequency;\n"
    );
}

#[test]
fn keeps_the_next_import_in_the_same_block() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export type Left = string;\nexport type Kept = boolean;\n".into(),
        ),
        (
            "src/other.ts".into(),
            "export type Other = number;\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Left, Kept } from './shared';\nimport type { Other } from './other';\ntype App = Left | Kept | Other;\n"
                .into(),
        ),
        (
            "src/again.ts".into(),
            "import type { Left } from './shared';\ntype Again = Left;\n".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| candidate.symbol_name == "Left")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import type { Kept } from './shared';\nimport type { Other } from './other';\nimport type { Left } from './types/src';\ntype App = Left | Kept | Other;\n"
    );
}

#[test]
fn keeps_multiline_imports_wrapped_when_splitting() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export type Left = string;\nexport type VeryLongKeptNameAlpha = boolean;\nexport type VeryLongKeptNameBeta = boolean;\nexport type VeryLongKeptNameGamma = boolean;\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import type {\n  Left,\n  VeryLongKeptNameAlpha,\n  VeryLongKeptNameBeta,\n  VeryLongKeptNameGamma,\n} from './shared';\ntype App = Left | VeryLongKeptNameAlpha;\n".into(),
        ),
        (
            "src/other.ts".into(),
            "import type { Left } from './shared';\ntype Other = Left;\n".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| candidate.symbol_name == "Left")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import type {\n  VeryLongKeptNameAlpha,\n  VeryLongKeptNameBeta,\n  VeryLongKeptNameGamma,\n} from './shared';\nimport type { Left } from './types/src';\n\ntype App = Left | VeryLongKeptNameAlpha;\n"
    );
}

#[test]
fn keeps_the_blank_line_after_a_split_import() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export type Left = string;\nexport type Kept = boolean;\n".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Left, Kept } from './shared';\n\ntype App = Left | Kept;\n".into(),
        ),
        (
            "src/other.ts".into(),
            "import type { Left } from './shared';\ntype Other = Left;\n".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .filter(|candidate| candidate.symbol_name == "Left")
    .collect::<Vec<_>>();
    let changes = plan_extraction_rewrite(&candidates, &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import type { Kept } from './shared';\nimport type { Left } from './types/src';\n\ntype App = Left | Kept;\n"
    );
}

#[test]
fn preserves_type_only_default_imports_when_splitting() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/shared.ts".into(),
            "export default interface Default { id: string } export type Shared = { id: string };"
                .into(),
        ),
        (
            "src/app.ts".into(),
            "import type Default, { Shared } from './shared'; type App = Default | Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert_eq!(
        changes["src/app.ts"],
        "import type Default from './shared';\nimport type { Shared } from './types/src';\n\n type App = Default | Shared;"
    );
}

#[test]
fn preserves_aliased_mixed_imports_through_barrel_reexports() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/schema.ts".into(),
            "export type A = { id: string };\nexport const runtime = 1;\n".into(),
        ),
        (
            "src/feature/index.ts".into(),
            "export { type A as PublicA, runtime } from './schema';\n".into(),
        ),
        (
            "src/app.ts".into(),
            concat!(
                "import { type PublicA as LocalA, runtime } from './feature';\n",
                "export const result: LocalA = { id: 'x' };\n",
                "export const count = runtime;\n",
            )
            .into(),
        ),
    ]);

    let candidate = apply_placement(
        scan_project_files(
            sources
                .iter()
                .map(|(path, source)| (path.clone(), source.clone()))
                .collect(),
        )
        .expect("scans"),
        PlacementStyle::Collocation,
    )
    .into_iter()
    .find(|candidate| candidate.symbol_name == "A")
    .expect("finds shared type");

    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans extraction");

    let destination = changes
        .get("src/feature/types/feature.ts")
        .expect("writes moved type destination");
    assert_eq!(destination, "export type A = { id: string };\n");

    let barrel = changes
        .get("src/feature/index.ts")
        .expect("rewrites barrel export");
    assert!(
        barrel.contains("export { runtime } from './schema';"),
        "{barrel}"
    );
    assert!(
        barrel.contains("export type { A as PublicA } from './types/feature';"),
        "{barrel}"
    );

    // The consumer continues importing through the public barrel; only the
    // barrel's type re-export needs to point at the extracted declaration.
    assert!(!changes.contains_key("src/app.ts"));

    compile_rewrite_fixture(&sources, &changes);
}

#[test]
fn preserves_value_imports_when_moving_mixed_type_only_shared() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/external.ts".into(),
            "export interface External { id: string }\nexport const value = 1;\n".into(),
        ),
        (
            "src/shared.ts".into(),
            [
                "import { value, type External } from './external';",
                "import type * as ExternalTypes from './external';",
                "export type Shared = External & ExternalTypes.External;",
                "void value;",
            ]
            .join("\n"),
        ),
        (
            "src/consumer.ts".into(),
            "import type { Shared } from './shared'; type Consumer = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(changes["src/shared.ts"].contains("import { value } from './external';"));
    assert!(changes["src/shared.ts"].contains("void value;"));
    assert!(changes["src/types/src.ts"].contains("import type * as ExternalTypes"));
    assert!(changes["src/types/src.ts"].contains("import type { External }"));
    assert!(changes["src/types/src.ts"].contains("export type Shared"));
}

#[test]
fn moves_type_only_imports_to_the_destination() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/external.ts".into(),
            "export type External = {};".into(),
        ),
        (
            "src/shared.ts".into(),
            "import type { External } from './external'; export type Shared = External;".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds shared");
    let changes = plan_extraction_rewrite(&[candidate], &sources).expect("plans");
    assert!(changes["src/shared.ts"].contains("export type { Shared } from './types/src';"));
    assert!(
        changes["src/types/src.ts"].starts_with("import type { External } from '../external';")
    );
}

#[test]
fn rejects_conflicting_declaration_symbols_for_one_destination() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        (
            "src/feature/one.ts".into(),
            "export type Shared = { one: string };".into(),
        ),
        (
            "src/feature/two.ts".into(),
            "export type Shared = { two: string };".into(),
        ),
        (
            "src/consumer-one.ts".into(),
            "import type { Shared } from './feature/one'; type ConsumerOne = Shared;".into(),
        ),
        (
            "src/consumer-two.ts".into(),
            "import type { Shared } from './feature/two'; type ConsumerTwo = Shared;".into(),
        ),
    ]);
    let candidates = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans");
    let error = plan_extraction_rewrite(&candidates, &sources)
        .expect_err("conflicts")
        .to_string();
    assert!(error.contains("Conflicting declarations named Shared"));
    assert!(error.contains("src/feature/one.ts#Shared"));
    assert!(error.contains("src/feature/two.ts#Shared"));
}

#[test]
fn rejects_unsafe_candidate_paths() {
    let sources: BTreeMap<String, String> = BTreeMap::from([
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ]);
    let mut candidate = scan_project_files(
        sources
            .iter()
            .map(|(path, source)| (path.clone(), source.clone()))
            .collect(),
    )
    .expect("scans")
    .remove(0);
    candidate.destination_path = "../outside.ts".into();
    assert!(plan_extraction_rewrite(&[candidate], &sources).is_err());
}
