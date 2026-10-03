import unittest
from check_imports import violations


class ImportBoundary(unittest.TestCase):
    def test_deliberate_platform_violation(self):
        for name in ('android.view.View', 'androidx.activity.ComponentActivity', 'java.nio.ByteBuffer', 'javax.imageio.ImageIO'):
            with self.subTest(name=name): self.assertEqual(violations(f'import {name}\n'), [name])

    def test_alias_comments_backticks_cannot_hide_import(self):
        self.assertEqual(violations('import java /* outer /* inner */ end */ . lang.String as Text'), ['java.lang.String'])
        self.assertEqual(violations('import `android`.view.View'), ['android.view.View'])

    def test_portable_dependencies(self):
        self.assertEqual(violations('import kotlinx.coroutines.flow.Flow\n// import java.io.File\n/* import android.view.View */'), [])

    def test_annotation_string_does_not_hide_later_import(self):
        self.assertEqual(violations('@file:Suppress("/*")\nimport java.io.File'), ['java.io.File'])
        self.assertEqual(violations('val sample = """\nimport java.io.File\n"""'), [])


if __name__ == '__main__': unittest.main()
