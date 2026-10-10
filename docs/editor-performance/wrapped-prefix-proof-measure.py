import runpy,json,sys
module=runpy.run_path('/workspace/repos/openwebide/tools/measure-editor-view.py')
Original=module['Browser']
PATCH=r"""
(() => {
 const native=Element.prototype.getBoundingClientRect;
 const seen=new WeakSet(), retained=new Map();
 window.probeIdentities={unique:0,withinPhaseMatches:0,previousPhaseMatches:0,samples:[]};
 Element.prototype.getBoundingClientRect=function(...args){
  const result=native.apply(this,args);
  if(this.matches('.editor-row-measure .editor-source-line') && !seen.has(this)) {
   seen.add(this);
   const owner=this.closest('.editor-row-measure');
   const phase=window.editorViewMeasurement?.phase;
   const key=owner.style.cssText+'|'+owner.getAttribute('data-measure-font')+'|'+owner.getAttribute('data-measure-prepared')+'|'+this.getAttribute('style')+'|'+this.innerHTML;
   const previous=retained.get(key);
   if(previous) {
    probeIdentities[previous.phase===phase?'withinPhaseMatches':'previousPhaseMatches']++;
    if(probeIdentities.samples.length<12) probeIdentities.samples.push({previous,current:{phase,start:this.getAttribute('data-source-start')}});
   } else {probeIdentities.unique++;retained.set(key,{phase,start:this.getAttribute('data-source-start')});}
  }
  return result;
 };
})();
"""
class Browser(Original):
 def call(self,method,path,body=None,session=True,timeout=30):
  if path=='/goog/cdp/execute' and body and body.get('cmd')=='Page.addScriptToEvaluateOnNewDocument':
   body={**body,'params':{**body['params'],'source':PATCH+body['params']['source']}}
  return super().call(method,path,body,session,timeout)
 def stop(self):
  try:
   if self.session: print(json.dumps({'diagnostic':'exact probe markup/style identities, observations only','identities':self.script('return window.probeIdentities')}),file=sys.stderr,flush=True)
  finally:super().stop()
module['measure'].__globals__['Browser']=Browser
for mode in ['local','remote']:
 print(json.dumps(module['measure']('styled-long-line',mode,True,True,1,'beginning')),flush=True)
