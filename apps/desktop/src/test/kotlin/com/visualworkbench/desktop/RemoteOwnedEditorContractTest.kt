package com.visualworkbench.desktop

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class RemoteOwnedEditorContractTest {
    private val run="a".repeat(32)
    private fun fixture()=listOf("M4_REMOTE_OWNED_EDITOR_V1",run,"b".repeat(64),"krita","31","9000","c".repeat(32),"101","41","9001","500000,500000,501000,500000,502000,500000","d".repeat(64))
    private fun read(values:List<String>)=RemoteIntegrationHost.ownedEditorKind(values,run,101uL,41u,9001uL,31,9000)
    @Test fun exactKindsRemainExplicitFixtureMetadata() {
        assertEquals("krita",read(fixture()))
        assertEquals("paint",read(fixture().toMutableList().also{it[3]="paint"}))
    }
    @Test fun staleParentSourceOrMalformedCoordinatesNeverSelectAnEditor() {
        val replacements=mapOf(0 to "other",1 to "e".repeat(32),2 to "bad",3 to "other",4 to "32",5 to "9001",6 to "bad",7 to "102",8 to "42",9 to "9002",10 to "500000,500000,501000,500000,503000,500000",11 to "bad")
        replacements.forEach{(index,value)->assertThrows(IllegalArgumentException::class.java){read(fixture().toMutableList().also{it[index]=value})}}
        assertThrows(IllegalArgumentException::class.java){read(fixture().dropLast(1))}
        assertThrows(IllegalArgumentException::class.java){read(fixture()+"extra")}
        assertThrows(IllegalArgumentException::class.java){read(fixture().toMutableList().also{it[10]="998000,500000,999000,500000,1000000,500000"})}
    }
}
