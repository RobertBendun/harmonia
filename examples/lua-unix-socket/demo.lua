harmonia = require "harmonia"

-- https://computermusicresource.com/midikeys.html
c2 = 48
e2 = 52
g2 = 55
c4 = 72
e4 = 76
g4 = 79

function play(note, duration)
	coroutine.yield("play", note, duration)
end

function sleep(duration)
	coroutine.yield("sleep", duration)
end

harmonia.bind_block("/tmp/harmonia-block.socket", function ()
	local prog = {c4, e4, g4}

	for i = 1,8 do
		for j = 1,i do
			local time = 2 ^ (1 - i)
			play(prog[i//#prog + 1], time)
			sleep(time)
		end
	end

	play(c2, 1)
	play(e2, 1)
	play(g2, 1)
end)
