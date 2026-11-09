//! Transparent LLVM codegen wrapper: count from entry through worker join.
#![feature(rustc_private)]

extern crate rustc_codegen_llvm;
extern crate rustc_codegen_ssa;
extern crate rustc_data_structures;
#[allow(unused_extern_crates)]
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_metadata;
extern crate rustc_middle;
extern crate rustc_session;

mod counter;
mod report;

use rustc_codegen_ssa::traits::CodegenBackend;
use rustc_codegen_ssa::{CodegenResults, TargetConfig};
use rustc_data_structures::fx::FxIndexMap;
use rustc_metadata::{EncodedMetadata, creader::MetadataLoaderDyn};
use rustc_middle::dep_graph::{WorkProduct, WorkProductId};
use rustc_middle::ty::TyCtxt;
use rustc_middle::util::Providers;
use rustc_session::Session;
use rustc_session::config::{CrateType, OutputFilenames, PrintRequest};
use std::any::Any;

struct ProfiledLlvm {
    inner: Box<dyn CodegenBackend>,
}

struct Ongoing {
    llvm: Box<dyn Any>,
    measurement: Option<report::SessionMeasurement>,
}

impl CodegenBackend for ProfiledLlvm {
    fn locale_resource(&self) -> &'static str {
        self.inner.locale_resource()
    }
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn init(&self, session: &Session) {
        self.inner.init(session);
    }
    fn print(&self, request: &PrintRequest, output: &mut String, session: &Session) {
        self.inner.print(request, output, session);
    }
    fn target_config(&self, session: &Session) -> TargetConfig {
        self.inner.target_config(session)
    }
    fn supported_crate_types(&self, session: &Session) -> Vec<CrateType> {
        self.inner.supported_crate_types(session)
    }
    fn print_passes(&self) {
        self.inner.print_passes();
    }
    fn print_version(&self) {
        self.inner.print_version();
    }
    fn has_zstd(&self) -> bool {
        self.inner.has_zstd()
    }
    fn metadata_loader(&self) -> Box<MetadataLoaderDyn> {
        self.inner.metadata_loader()
    }
    fn provide(&self, providers: &mut Providers) {
        self.inner.provide(providers);
    }

    fn codegen_crate<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Box<dyn Any> {
        let measurement = report::SessionMeasurement::start(
            tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE).to_string(),
        );
        let llvm = self.inner.codegen_crate(tcx);
        Box::new(Ongoing { llvm, measurement })
    }

    fn join_codegen(
        &self,
        ongoing: Box<dyn Any>,
        session: &Session,
        outputs: &OutputFilenames,
    ) -> (CodegenResults, FxIndexMap<WorkProductId, WorkProduct>) {
        let ongoing = *ongoing
            .downcast::<Ongoing>()
            .expect("LLVM profiler received foreign ongoing codegen");
        let result = self.inner.join_codegen(ongoing.llvm, session, outputs);
        if let Some(measurement) = ongoing.measurement {
            measurement.finish();
        }
        result
    }

    fn link(
        &self,
        session: &Session,
        results: CodegenResults,
        metadata: EncodedMetadata,
        outputs: &OutputFilenames,
    ) {
        self.inner.link(session, results, metadata, outputs);
    }
}

#[unsafe(no_mangle)]
pub fn __rustc_codegen_backend() -> Box<dyn CodegenBackend> {
    Box::new(ProfiledLlvm {
        inner: rustc_codegen_llvm::LlvmCodegenBackend::new(),
    })
}
