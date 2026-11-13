# Deep-but-legal nesting (0.9.1): the parser limits are 200 levels, so 100
# levels of each construct must still parse and run. The target 1 MiB user
# stack bounds the *effective* bracket depth to ~186 (see README), far above
# this file.

x = ((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((1))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))
assert x == 1

y = not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not not True
assert y is True

z = 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 1 if True else 0
assert z == 1

f = lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: lambda: 42
for _ in range(100):
    f = f()
assert f == 42

# 100 nested if blocks, one space of indentation per level
if True:
 if True:
  if True:
   if True:
    if True:
     if True:
      if True:
       if True:
        if True:
         if True:
          if True:
           if True:
            if True:
             if True:
              if True:
               if True:
                if True:
                 if True:
                  if True:
                   if True:
                    if True:
                     if True:
                      if True:
                       if True:
                        if True:
                         if True:
                          if True:
                           if True:
                            if True:
                             if True:
                              if True:
                               if True:
                                if True:
                                 if True:
                                  if True:
                                   if True:
                                    if True:
                                     if True:
                                      if True:
                                       if True:
                                        if True:
                                         if True:
                                          if True:
                                           if True:
                                            if True:
                                             if True:
                                              if True:
                                               if True:
                                                if True:
                                                 if True:
                                                  if True:
                                                   if True:
                                                    if True:
                                                     if True:
                                                      if True:
                                                       if True:
                                                        if True:
                                                         if True:
                                                          if True:
                                                           if True:
                                                            if True:
                                                             if True:
                                                              if True:
                                                               if True:
                                                                if True:
                                                                 if True:
                                                                  if True:
                                                                   if True:
                                                                    if True:
                                                                     if True:
                                                                      if True:
                                                                       if True:
                                                                        if True:
                                                                         if True:
                                                                          if True:
                                                                           if True:
                                                                            if True:
                                                                             if True:
                                                                              if True:
                                                                               if True:
                                                                                if True:
                                                                                 if True:
                                                                                  if True:
                                                                                   if True:
                                                                                    if True:
                                                                                     if True:
                                                                                      if True:
                                                                                       if True:
                                                                                        if True:
                                                                                         if True:
                                                                                          if True:
                                                                                           if True:
                                                                                            if True:
                                                                                             if True:
                                                                                              if True:
                                                                                               if True:
                                                                                                if True:
                                                                                                 if True:
                                                                                                  if True:
                                                                                                   if True:
                                                                                                    marker = 1
assert marker == 1
print('NESTING OK')
