import contextlib,copy,io,json,runpy,sys,tempfile
from pathlib import Path
script=Path(__file__).with_name('gate_table.py')
with tempfile.TemporaryDirectory() as d:
    sys.argv=[str(script),d]
    with contextlib.redirect_stdout(io.StringIO()):
        ns=runpy.run_path(str(script))
check=ns['correctness']
base={'status':'ok','driver':{'exit_code':0},'corpus':{'exit_code':0,'sha256':'abc','lines':3},'workload_check':{k:{'exit_code':0,'checksum':17} for k in ['parse','req','identifier']},'tests':{'itest':{'binaries':{'test':{'outcomes':{'case':'ok'}}}}}}
cases={}
def run(name,rec,prefix):
 status,notes=check(base,rec,'semver')
 assert status.startswith(prefix),(name,status,notes)
 cases[name]={'status':status,'notes':notes}
run('complete',copy.deepcopy(base),'pass')
r=copy.deepcopy(base); del r['driver']; del r['corpus']; del r['workload_check']; run('missing_driver_corpus_checks',r,'incomplete')
r=copy.deepcopy(base); del r['workload_check']['req']; run('missing_workload',r,'incomplete')
r=copy.deepcopy(base); r['workload_check']['parse']['exit_code']=1; r['workload_check']['parse']['checksum']=None; run('failed_workload',r,'FAIL: output')
r=copy.deepcopy(base); r['corpus']['sha256']='different'; run('wrong_output',r,'FAIL: output')
r=copy.deepcopy(base); del r['tests']; run('missing_tests',r,'incomplete')
r=copy.deepcopy(base); r['tests']['itest']['binaries']['test']['outcomes']['case']='failed'; run('failed_test',r,'FAIL: tests')
r=copy.deepcopy(base); r['tests']['itest']={'build_error':['unsupported intrinsic']}; run('unbuilt_tests',r,'incomplete')
# Results are checked in memory; do not write into the source tree.
print('PASS 8 gate report cases, including missing driver/corpus/workload and lost LLVM-passing tests')
