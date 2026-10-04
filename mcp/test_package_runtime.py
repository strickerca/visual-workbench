import hashlib
from pathlib import Path
import tempfile
import unittest
import package_runtime as runtime

class PackageRuntimeTests(unittest.TestCase):
    def tree(self, root, names):
        for name in names:
            path=root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(b"fixture")
    def test_server_exact_bytes_and_exclusive_destination(self):
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp);stage=base/"stage";stage.mkdir();self.tree(stage,runtime.SERVER_REQUIRED|{"mcp/node_modules/sdk/index.js"})
            result=runtime.server(stage,base/"resources")
            self.assertEqual(result,(base/"resources/vw-mcp-server.sha256").read_bytes())
            for hash_,size,name in runtime.scan(stage):
                body=(base/"resources/mcp-server"/name).read_bytes();self.assertEqual(size,len(body));self.assertEqual(hash_,hashlib.sha256(body).hexdigest())
            with self.assertRaises(FileExistsError):runtime.server(stage,base/"resources")
    def test_missing_runtime_component_refuses_before_publication(self):
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp);stage=base/"stage";stage.mkdir();self.tree(stage,runtime.SERVER_REQUIRED-{"node.exe"})
            with self.assertRaises(ValueError):runtime.server(stage,base/"resources")
            self.assertFalse((base/"resources").exists())
    def test_app_image_has_no_self_hash_cycle_and_does_not_overwrite(self):
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp);image=base/"image";image.mkdir();self.tree(image,{"VisualWorkbenchDev.exe","app/main.jar","runtime/bin/java.dll"})
            data=runtime.app_image(image,base/"embedded.txt","VisualWorkbenchDev.exe")
            self.assertEqual(data,(image/"vw-app-image.sha256").read_bytes());self.assertNotIn(b"vw-app-image",data);self.assertNotIn(b"vw-mcp.exe",data)
            with self.assertRaises(ValueError):runtime.app_image(image,base/"new.txt","VisualWorkbenchDev.exe")
            self.assertEqual(data,(image/"vw-app-image.sha256").read_bytes())
    def test_parent_traversal_aliases_ads_and_reserved_files_refuse(self):
        for name in ["../x","/x","a//b","a\\b","a:b","a/NUL.txt","a/COM1","a./x","a /x"]:
            with self.subTest(name=name),self.assertRaises(ValueError):runtime.path_name(name)
    def test_case_colliding_inventory_refuses(self):
        # The parser also enforces this on Windows case-insensitive filesystems.
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp);self.tree(base,{"same","SAME"})
            if len(list(base.iterdir()))==2:
                with self.assertRaises(ValueError):runtime.scan(base)
            else:self.assertEqual(1,len(runtime.scan(base)))
    def test_manifest_admission_precedes_large_retention(self):
        with self.assertRaises(ValueError):runtime.encoded([])
        with self.assertRaises(ValueError):runtime.path_name("x/"*33+"x")
    def test_app_launcher_without_jre_or_jar_is_not_a_working_bundle(self):
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp);image=base/"image";image.mkdir();self.tree(image,{"VisualWorkbenchDev.exe"})
            with self.assertRaises(ValueError):runtime.app_image(image,base/"manifest","VisualWorkbenchDev.exe")
            self.assertFalse((image/"vw-app-image.sha256").exists())
    def test_server_never_embeds_bridge_or_its_own_application_jar(self):
        for extra in ["vw-mcp.exe","app/main.jar","VisualWorkbenchDev.exe"]:
            with tempfile.TemporaryDirectory() as tmp:
                base=Path(tmp);stage=base/"stage";stage.mkdir();self.tree(stage,runtime.SERVER_REQUIRED|{extra})
                with self.assertRaises(ValueError):runtime.server(stage,base/"resources")
                self.assertFalse((base/"resources").exists())
    def test_two_launchers_refuse_before_inventory_publication(self):
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp);image=base/"image";image.mkdir();self.tree(image,{"VisualWorkbench.exe","VisualWorkbenchDev.exe","app/main.jar","runtime/bin/java.dll"})
            with self.assertRaises(ValueError):runtime.app_image(image,base/"manifest","VisualWorkbenchDev.exe")
            self.assertFalse((image/"vw-app-image.sha256").exists());self.assertFalse((base/"manifest").exists())

if __name__=="__main__":unittest.main()
