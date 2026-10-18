package com.visualworkbench.shared

import kotlin.test.*

class RemoteAuthorityObservationTest {
    private val binding=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,"input")
    private val digest="a".repeat(64)
    private val canvas="""{"runtime_id_hash":"${"b".repeat(64)}","process_id":7,"sampled_qpc_100ns":101,"class_name":"KisOpenGLCanvas2","control_type":50026,"canvas_rect":{"x":0,"y":0,"width":800,"height":600},"profile_digest":"$digest"}"""
    private fun json(focus:String="null",selected:String=canvas,routes:String="essential"):String =
        """{"identity":{"executableName":"krita.exe","executableBlake3":"${"c".repeat(64)}","executableBytes":1024,"fileVersion":"5.3.4.0","packageFullName":null,"packageVersion":null},"profile_digest":"$digest","verified_actions":[1,2,3,6,7,8,9],"routes":"$routes","focus":$focus,"selected_canvas":$selected}"""
    private fun parse(text:String):RemoteAuthorityObservation=remoteAuthorityObservation(RemoteNativeJson.read(text),binding)
    @Test fun selectedObservationNeverInventsKeyboardFocus() {
        val selected=parse(json())
        assertNull(selected.focus);assertNotNull(selected.selected)
        assertEquals(binding.targetToken,selected.selected.targetToken)
        val focused=parse(json(focus=canvas,selected="null",routes="legacy"))
        assertNotNull(focused.focus);assertNull(focused.selected)
    }
    @Test fun dualMissingAndUnknownObservationVariantsAreRefused() {
        for(text in listOf(
            json(focus=canvas),json(selected="null"),
            json().replace("\"focus\":null,",""),
            json(focus=canvas,selected="null").replace(",\"selected_canvas\":null",""),
            json().dropLast(1)+",\"guessed_focus\":true}",
        ))assertFails{parse(text)}
        assertFails{parse(json().replace("\"focus\":null","\"focus\":null,\"focus\":null"))}
    }
    @Test fun selectedObservationCannotAuthorizeLegacyOrExtendedActions() {
        assertFails{parse(json(routes="legacy"))}
        assertFails{parse(json(routes="unknown"))}
        assertFails{parse(json().replace("[1,2,3,6,7,8,9]","[1,11]"))}
        assertFails{parse(json(focus=canvas,selected="null").replace("[1,2,3,6,7,8,9]","[1,11]"))}
    }
    @Test fun staleDigestMalformedGeometryAndDuplicateActionsAreRefused() {
        for(text in listOf(
            json().replace("\"process_id\":7","\"process_id\":0"),
            json().replace("\"sampled_qpc_100ns\":101","\"sampled_qpc_100ns\":0"),
            json().replace("\"width\":800","\"width\":0"),
            json().replace("[1,2,3,6,7,8,9]","[1,1]"),
            json(selected=canvas.replace(digest,"d".repeat(64))),
            json(selected=canvas.dropLast(1)+",\"focus_claim\":true}"),
        ))assertFails{parse(text)}
    }
}
