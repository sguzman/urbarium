# Urbarium Constitution

## Scope and independence

Urbarium is a public geographic reconstruction and procedural 3D exploration project. It transforms attributed geographic source features into viewer and exportable scene geometry. The existing Downtown Los Angeles pilot and its named regional sections are partial prototypes, not certified architectural replicas or complete urban coverage.

All core code, documented source interfaces, versioned geography definitions, and public presentation belong to this independent repository. Urbarium is not a component of a monorepo and must not require private data sources or unpublished project dependencies. Other public registries may link to Urbarium without copying its implementation.

## Geographic evidence and accuracy

Document data provenance, collection method, geographic coverage, vintage, units, relevant licenses and provider limitations. Preserve the difference between measured values and explicitly marked rendering approximations. WGS84 bounding rectangles are geographic work envelopes, not surveyed city limits. Building footprints and centerlines are not automatically accurate buildings or navigable pavements. Experimental freeway geometry must not be represented as verified.

Do not publish source responses, restricted geographic datasets, private records, inaccessible assets or source code whose release rights are unclear. Respect source-specific access and caching policies.

## Public information boundary

Never publish private correspondence, personal writing, unpublished records, credentials, private project names or links, nonpublic source paths, or recognizable paraphrases of private discussions. The rule applies to source, code comments, documentation, examples, issues, assets, build outputs, logs, commit messages, and history. Public descriptions should remain independent, neutral and technical. If provenance or permission is uncertain, exclude the material.

## Ontology and gallery

Maintain human-readable and machine-readable records of geographic regions, spatial features, data sources, coordinate systems, transformation processes and presentation outputs. Keep the machine-readable ontology at [ontology/ontology.json](ontology/ontology.json), with accompanying explanations at [ontology/README.md](ontology/README.md). The public gallery at [gallery/](gallery/) indexes existing demonstrations and reports their limitations, independently of which renderer currently displays them.

## Membership and authority

The repository owner alone authorizes enrollment in any external registry, including Tarium. Public visibility, use of compatible governance documents, and ongoing project work do not grant an agent permission to change a separate registry. Do not add other repositories to Tarium or process project queues without a separate explicit instruction.

## Engineering changes

Work on the current `main` branch unless an explicit scoped exception is given. Preserve existing viewer behavior, documented source paths and public data contracts. Validate changed Rust, WebAssembly, static-site and geography components proportionately, and report unverified behavior without implying a successful deployment or verified geographic reconstruction.
