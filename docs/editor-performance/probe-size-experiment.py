import runpy,json,sys
module=runpy.run_path('/workspace/repos/openwebide/tools/measure-editor-view.py')
Original=module['Browser']
PATCH=r"""
(() => {
 const native=Object.getOwnPropertyDescriptor(Element.prototype,'innerHTML');
 Object.defineProperty(Element.prototype,'innerHTML',{...native,set(html){
  if(html && this.matches('.editor-row-measure > .editor-highlight-content')){
   window.lastProbeParent=this.parentElement.cloneNode(false);
   window.lastProbePaint=this.cloneNode(false);
  }
  return native.set.call(this,html);
 }});
})();
"""

class Browser(Original):
 def call(self,method,path,body=None,session=True,timeout=30):
  if path=='/goog/cdp/execute' and body and body.get('cmd')=='Page.addScriptToEvaluateOnNewDocument':
   body={**body,'params':{**body['params'],'source':PATCH+body['params']['source']}}
  return super().call(method,path,body,session,timeout)
 def stop(self):
  try:
   if self.session:
    results=self.script(r"""
      const parent=window.lastProbeParent, paint=window.lastProbePaint;
      parent.appendChild(paint);document.body.appendChild(parent);
      const results=[];
      try {
       for(const budget of [2048,4096,8192,16384]) {
        const units=Math.floor(budget/14), text='文😀 words '.repeat(units);
        let content='';
        for(let i=0;i<units;i+=35)content+='<span class="editor-text-run">'+ '文😀 words '.repeat(Math.min(35,units-i))+'</span>';
        for(let iteration=0;iteration<24;iteration++){
         const origin=[0,61.125,185.1875][iteration%3];
         const html='<span class="editor-source-line"><span style="display:inline-block;width:'+origin+'px"></span><span class="tok-string">'+content+'</span></span>';
         const started=performance.now();paint.innerHTML=html;
         const before=performance.now();const row=paint.firstElementChild;
         const bounds=row.getBoundingClientRect();
         results.push({budget,sourceBytes:units*14,iteration,origin,mutationMs:before-started,layoutMs:performance.now()-before,width:bounds.width,height:bounds.height});
        }
       }
      } finally {parent.remove();}
      return {experiment:'wrapped probe size reflow; diagnostic only', font:parent.style.fontFamily, results};
    """)
    print(json.dumps(results),file=sys.stderr,flush=True)
  finally:super().stop()
module['measure'].__globals__['Browser']=Browser
for mode in ['local']:
 print(json.dumps(module['measure']('styled-long-line',mode,True,True,1,'beginning')),flush=True)
